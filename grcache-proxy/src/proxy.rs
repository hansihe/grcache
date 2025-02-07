use std::time::{Duration, SystemTime};

use anyhow::bail;
use async_trait::async_trait;
use blake2::{Blake2b, Digest};
use grcache_shared::service::MethodSpec;
use pingora::{
    cache::{CacheKey, CacheMeta, RespCacheable, Storage},
    http::ResponseHeader,
    prelude::HttpPeer,
    protocols::ALPN,
};
use pingora_proxy::{ProxyHttp, Session};

use crate::service_store::ServiceData;

pub struct GrpcProxy {
    pub service_config: crate::service_store::ServiceConfig,
    pub cache: &'static (dyn Storage + Sync),
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

#[derive(Default)]
pub struct RequestCtx {
    do_cache: bool,
    grpc_meta: Option<GrpcMeta>,
}

pub(crate) type Blake2b128 = Blake2b<blake2::digest::consts::U16>;

#[async_trait]
impl ProxyHttp for GrpcProxy {
    type CTX = RequestCtx;
    fn new_ctx(&self) -> Self::CTX {
        RequestCtx::default()
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

        ctx.grpc_meta = Some(GrpcMeta {
            service_data: service_spec.clone(),
            method_name: method.into(),
            //request_message: None,
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
                "cache enabled for request with ttl: {}",
                cache_spec.descriptor.cache_ttl
            );

            session.enable_retry_buffering();
            // TODO no panic on no body
            while let Some(_bytes) = session.read_request_body().await? {}
            //session.read_body_or_idle(false).await?.unwrap();

            //session.as_http2().unwrap().read_trailers().await;

            if session.retry_buffer_truncated() {
                log::error!("request body above buffer size!");
                return Err(pingora::Error::explain(
                    pingora::ErrorType::HTTPStatus(413),
                    "request body size above buffer size for cache",
                ));
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

        Ok(false)
    }

    fn request_cache_filter(
        &self,
        session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> pingora::Result<()> {
        if ctx.do_cache {
            // TODO cache lock handler
            session.cache.enable(self.cache, None, None, None);
        }

        Ok(())
    }

    fn cache_key_callback(
        &self,
        session: &Session,
        _ctx: &mut Self::CTX,
    ) -> pingora::Result<CacheKey> {
        let request_body = session.get_retry_buffer().unwrap();

        let mut hasher = Blake2b128::new();

        let raw_path = session.req_header().raw_path();
        hasher.update(raw_path.len().to_le_bytes());
        hasher.update(raw_path);

        // TODO we can be smarter with how we hash messages to
        // eliminate encoding discrepancies.
        // Protobuf messages do not have a canonical serialization
        // format.
        hasher.update(request_body.len().to_le_bytes());
        hasher.update(request_body);

        let hash = hasher.finalize();

        let req_header = session.req_header();
        let mut cache_key = CacheKey::default(req_header);
        cache_key.set_primary_bin_override(hash.into());

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

        let age_sec = meta.age().as_secs();
        let hit = (age_sec as i64) < (cache_spec.descriptor.cache_ttl as i64);
        log::info!("found cache entry (age: {}s) (hit: {})", age_sec, hit);
        Ok(hit)
    }

    async fn response_trailer_filter(
        &self,
        _session: &mut Session,
        _upstream_trailers: &mut http::HeaderMap,
        _ctx: &mut Self::CTX,
    ) -> pingora::Result<Option<bytes::Bytes>>
    where
        Self::CTX: Send + Sync,
    {
        Ok(None)
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

            Ok(Box::new(peer))
        } else {
            Err(pingora::Error::explain(
                pingora::ErrorType::HTTPStatus(502),
                "no upstream available",
            ))
        }
    }

    async fn logging(
        &self,
        _session: &mut Session,
        e: Option<&pingora::Error>,
        _ctx: &mut Self::CTX,
    ) where
        Self::CTX: Send + Sync,
    {
        if let Some(error) = e {
            log::info!("request error: {}", error);
        } else {
            log::info!("request OK");
        }
    }
}
