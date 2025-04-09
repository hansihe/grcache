use std::{
    collections::BTreeSet,
    time::{Duration, SystemTime},
};

use anyhow::bail;
use async_trait::async_trait;
use blake2::{Blake2b, Digest};
use bytes::Bytes;
use grcache_shared::service::MethodSpec;
use opentelemetry::{
    global::{BoxedSpan, BoxedTracer},
    trace::{noop::NoopTracer, Span as _, SpanKind, Tracer},
    KeyValue,
};
use pingora::{
    cache::{CacheKey, CacheMeta, RespCacheable},
    http::ResponseHeader,
    prelude::HttpPeer,
    protocols::ALPN,
};
use pingora_proxy::{ProxyHttp, Session};

use crate::{
    cache::GrcacheStorage,
    grpc::{
        hash::{hash_body, hash_vary},
        headers::{find_strip_headers, make_vary_headers_set},
        status::GrpcStatus,
    },
    service_store::{ServiceConfig, ServiceData},
    tracing::extract_context_from_headers,
};

//mod logic;

pub struct GrpcProxy {
    pub service_config: crate::service_store::ServiceConfig,
    pub cache: &'static (dyn GrcacheStorage + Sync),
    pub noop_tracer: BoxedTracer,
    pub tracer: BoxedTracer,
}

impl GrpcProxy {
    pub fn new(service_config: ServiceConfig, cache: &'static (dyn GrcacheStorage + Sync)) -> Self {
        GrpcProxy {
            service_config,
            cache,
            noop_tracer: BoxedTracer::new(Box::new(NoopTracer::new())),
            tracer: opentelemetry::global::tracer("grcache-proxy"),
        }
    }

    fn new_ctx(&self) -> RequestCtx {
        RequestCtx {
            span: self.noop_tracer.start("request"),
            do_cache: false,
            grpc_meta: None,
        }
    }
}

fn parse_grpc_path(path: &[u8]) -> Result<(&str, &str), anyhow::Error> {
    let path = std::str::from_utf8(path)?;
    let mut parts = path.split("/");

    if parts.next() != Some("") {
        bail!("path must lead with /");
    }

    let service = parts.next();
    let method = parts.next();

    if method.is_none() || parts.next().is_some() {
        bail!("path must have exactly 2 elements for gRPC");
    }

    Ok((service.unwrap(), method.unwrap()))
}

struct GrpcMeta {
    service_data: ServiceData,
    method_name: String,
    vary_set: BTreeSet<String>,
    //request_message: Option<Box<dyn MessageDyn>>,
}

impl GrpcMeta {
    fn method_spec(&self) -> Option<&MethodSpec> {
        self.service_data
            .service_spec
            .as_ref()
            .and_then(|s| s.methods.get(&self.method_name))
    }
}

pub struct RequestCtx {
    span: BoxedSpan,
    do_cache: bool,
    grpc_meta: Option<GrpcMeta>,
}

pub(crate) type Blake2b128 = Blake2b<blake2::digest::consts::U16>;

// Ordering:
// + ->P Read request header
// - async `request_filter`
// - ? `request_cache_filter`
//   if `session.cache.enabled()`
//   - `cache_key_callback`
//   - `is_purge`
//   - cache bypass prediction `cache.cacheable_prediction`
//   -
// - async `upstream_peer`
// + P-> Connect to upstream
// + P-> Send request to upstream
// + P<- Receive upstream response
// - `upstream_response_trailer_filter`
// - AFTER h2 EOS `finish_miss_handler` -> `MissHandler::finish`
// + <-PSend downstream response
//
// ?
// async `cache_hit_filter`
// `response_cache_filter`

#[async_trait]
impl ProxyHttp for GrpcProxy {
    type CTX = RequestCtx;
    fn new_ctx(&self) -> Self::CTX {
        self.new_ctx()
    }

    async fn early_request_filter(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> pingora::Result<()>
    where
        Self::CTX: Send + Sync,
    {
        // Extract trace context from request
        let trace_ctx = extract_context_from_headers(&session.req_header().headers);

        // Create span for request handling
        ctx.span = self
            .tracer
            .span_builder("request")
            .with_kind(SpanKind::Server)
            .start_with_context(&self.tracer, &trace_ctx);

        Ok(())
    }

    async fn request_filter(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> pingora::Result<bool>
    where
        Self::CTX: Send + Sync,
    {
        let (service_name, method) =
            parse_grpc_path(session.req_header().raw_path()).map_err(|cause| {
                pingora::Error::because(
                    pingora::ErrorType::HTTPStatus(400),
                    "invalid grpc http path",
                    cause,
                )
            })?;

        ctx.span
            .set_attribute(KeyValue::new("grpc_service", service_name.to_owned()));
        ctx.span
            .set_attribute(KeyValue::new("grpc_method", method.to_owned()));

        let service_spec = {
            let services_map = self.service_config.services.pin();

            services_map
                .get(service_name)
                // Service is only present if we have a LB
                .filter(|s| s.load_balancer.is_some())
                .ok_or_else(|| {
                    pingora::Error::explain(pingora::ErrorType::HTTPStatus(404), "unknown service")
                })?
                .clone()
        };

        if let Some(service) = service_spec.service_spec.as_ref() {
            // Log a warning IF BOTH:
            // * The service was not marked as passthrough
            // * The method map does not contain the method
            //
            // We still proxy to upstream, but this is worrying and might
            // indicate out of date proto descriptors.
            if !service.passthrough && !service.methods.contains_key(method) {
                log::warn!(
                    "Got request for unknown gRPC method `{}` in service `{}`, passing through. Are the proto descriptors up to date?",
                    method,
                    service_name
                )
            }
        }

        let vary_set = make_vary_headers_set(&session.req_header().headers)?;

        ctx.grpc_meta = Some(GrpcMeta {
            service_data: service_spec.clone(),
            method_name: method.into(),
            vary_set,
        });

        if let Some((_method_spec, cache_spec)) = ctx
            .grpc_meta
            .as_ref()
            .unwrap()
            .method_spec()
            .and_then(|m| m.cache_spec.as_ref().map(|c| (m, c)))
            .filter(|(_m, c)| c.descriptor.cache_ttl > 0)
        {
            ctx.do_cache = true;
            log::info!(
                "Cache enabled for request with ttl: {}",
                cache_spec.descriptor.cache_ttl
            );

            session.enable_retry_buffering();
            while let Some(_bytes) = session.read_request_body().await? {}

            if session.retry_buffer_truncated() {
                log::error!("Request body above buffer size!");
                return Err(pingora::Error::explain(
                    pingora::ErrorType::HTTPStatus(413),
                    "request body size above buffer size for cache",
                ));
            }

            // Remove request headers which are not allowed.
            // We do this to protect against cache leaks.
            let strip_headers = find_strip_headers(
                &session.req_header().headers,
                &ctx.grpc_meta.as_ref().unwrap().vary_set,
                // TODO from config
                &BTreeSet::new(),
            );
            for header in strip_headers.iter() {
                session.req_header_mut().remove_header(header);
            }

            //let request_body = session.get_retry_buffer().unwrap();

            //let message_result = method_spec
            //    .descriptor
            //    .input_type()
            //    .parse_from_bytes(&request_body);

            //match message_result {
            //    Ok(message) => todo!(),
            //    Err(error) => {
            //        log::error!("error while parsing message for caching! {}", error);
            //    }
            //}
        } else {
            log::info!("cache not enabled for request");
        }

        ctx.span
            .set_attribute(KeyValue::new("cache_enabled", ctx.do_cache));

        Ok(false)
    }

    fn request_cache_filter(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> pingora::Result<()> {
        if ctx.do_cache {
            // Cache lock handler?
            // Likely inrrelevant for most backends.
            session
                .cache
                .enable(self.cache.as_storage(), None, None, None);
        }

        Ok(())
    }

    fn cache_key_callback(
        &self,
        session: &Session,
        ctx: &mut Self::CTX,
    ) -> pingora::Result<CacheKey> {
        let meta = ctx.grpc_meta.as_ref().unwrap();

        let req_header = session.req_header();
        let request_body = session.get_retry_buffer().unwrap();

        let mut hasher = Blake2b128::new();
        hash_vary(&mut hasher, &meta.vary_set, &req_header.headers);
        hash_body(&mut hasher, &req_header, &request_body);
        let key_hash = hasher.finalize();

        // Ultimately the only thing that matters here is that
        // how we construct the key here matches what the cache
        // backends expect.
        // For now the cache backends expect:
        // * A namespace. Empty for now.
        // * A primary bin override, used as primary cache key.
        let mut cache_key = CacheKey::new("", "", "");
        cache_key.set_primary_bin_override(key_hash.into());
        Ok(CacheKey::default(req_header))
    }

    async fn cache_hit_filter(
        &self,
        _session: &Session,
        meta: &CacheMeta,
        ctx: &mut Self::CTX,
    ) -> pingora::Result<bool>
    where
        Self::CTX: Send + Sync,
    {
        // TODO this is shit, make method
        let cache_spec = ctx
            .grpc_meta
            .as_ref()
            .unwrap()
            .method_spec()
            .unwrap()
            .cache_spec
            .as_ref()
            .unwrap();

        ctx.span.set_attribute(KeyValue::new("cache_hit", true));

        let age_sec = meta.age().as_secs();
        let hit = (age_sec as i64) < (cache_spec.descriptor.cache_ttl as i64);
        log::info!("found cache entry (age: {}s) (hit: {})", age_sec, hit);
        Ok(hit)
    }

    fn upstream_response_body_filter(
        &self,
        session: &mut Session,
        _body: &mut Option<Bytes>,
        end_of_stream: bool,
        _ctx: &mut Self::CTX,
    ) {
        if end_of_stream {
            log::error!("Request closed without trailers! Forwarding but not caching.");
            session.cache.disable(pingora::cache::NoCacheReason::Custom(
                "no trailers in request",
            ));
        }
    }

    // Called after we have gotten trailers from upstream, but
    // before response is cached.
    // This is where we take the `gRPC` error code into account.
    // At some point we might want to make the behaviour here
    // customizable, but for now we only cache non error responses.
    fn upstream_response_trailer_filter(
        &self,
        session: &mut Session,
        upstream_trailers: &mut http::HeaderMap,
        _ctx: &mut Self::CTX,
    ) -> pingora::Result<()> {
        match GrpcStatus::parse(upstream_trailers) {
            Err(error) => {
                log::warn!(
                    "Error while reading status from `gRPC` response trailers (forwarding but not caching): {}", error
                );
                session
                    .cache
                    .disable(pingora::cache::NoCacheReason::Custom("invalid trailers"));
                Ok(())
            }
            Ok(status) => {
                if !status.is_ok() {
                    session.cache.disable(pingora::cache::NoCacheReason::Custom(
                        "non-cachable trailers",
                    ));
                }
                Ok(())
            }
        }
    }

    fn response_cache_filter(
        &self,
        _session: &Session,
        resp: &ResponseHeader,
        _ctx: &mut Self::CTX,
    ) -> pingora::Result<RespCacheable> {
        // Even through we return cachable here, it doesn't mean we can
        // actually cache. Trailers also need to be checked for errors.
        Ok(RespCacheable::Cacheable(CacheMeta::new(
            SystemTime::now()
                .checked_add(Duration::from_secs(60))
                .unwrap(),
            SystemTime::now(),
            // TODO what are these?
            10,
            10,
            resp.clone(),
        )))
    }

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> pingora::Result<Box<HttpPeer>> {
        let backend = ctx
            .grpc_meta
            .as_ref()
            .unwrap()
            .service_data
            .load_balancer
            .as_ref()
            .unwrap()
            .load_balancer_round_robin
            // Hash doesn't matter for now.
            .select(b"", 256);

        if let Some(backend) = backend {
            let inet = backend.addr.as_inet().unwrap();

            let mut peer = HttpPeer::new(
                inet,
                false,
                // TODO set SNI properly? not terribly relevant for non tls grpc
                "".into(),
            );
            peer.options.alpn = ALPN::H2;
            //peer.options.verify_cert = false;

            ctx.span
                .set_attribute(KeyValue::new("upstream", inet.to_string()));

            Ok(Box::new(peer))
        } else {
            Err(pingora::Error::explain(
                pingora::ErrorType::HTTPStatus(502),
                "no upstream available",
            ))
        }
    }

    async fn logging(&self, _session: &mut Session, e: Option<&pingora::Error>, ctx: &mut Self::CTX)
    where
        Self::CTX: Send + Sync,
    {
        if let Some(error) = e {
            ctx.span.record_error(error);
            ctx.span.set_status(opentelemetry::trace::Status::Error {
                description: "failed while processing request".into(),
            });
            log::info!("request error: {}", error);
        } else {
            log::info!("request OK");
        }
        ctx.span.end();
    }
}
