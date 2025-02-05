use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
};

use anyhow::bail;
use async_trait::async_trait;
use cache::{local::LocalCacheBackend, redis_replicas::RedisReplicasCacheBackend};
use grcache_shared::service::MethodSpec;
use kube::Client;
use pingora::{
    apps::HttpServerOptions,
    cache::{CacheKey, Storage},
    prelude::HttpPeer,
    protocols::ALPN,
    services::background::GenBackgroundService,
};
use pingora_core::{prelude::Opt, server::Server};
use pingora_proxy::{ProxyHttp, Session};
use service_store::ServiceData;

pub mod cache;
pub mod discovery;
pub mod service_store;

fn main() {
    env_logger::init();

    //let boot_runtime = tokio::runtime::Builder::new_current_thread()
    //    .build()
    //    .expect("failed to build boot tokio runtime");
    //let k8s_client = boot_runtime.block_on(async {
    //    let k8s_client = Client::try_default()
    //        .await
    //        .expect("failed to create kubernetes client");
    //    k8s_client
    //});

    let opt = Opt::parse_args();
    // Daemonization is not supported due to how we create resources before
    // calling `run_forever`.
    assert!(!opt.daemon, "daemonization not supported");

    let mut server = Server::new(Some(opt)).unwrap();
    server.bootstrap();

    // DNS discovery background service
    let (dns_service, dns_discovery) = discovery::dns::Service::new();
    server.add_service(GenBackgroundService::new(
        format!("DNS Discovery Service"),
        Arc::new(dns_service),
    ));

    let (k8s_config_service, service_config) =
        service_store::K8SConfigService::new(dns_discovery.clone());
    server.add_service(GenBackgroundService::new(
        format!("Kubernetes Config Service"),
        Arc::new(k8s_config_service),
    ));

    let redis_discovery = dns_discovery.backends_for_hostname("redis-services".into(), 10001);
    let (redis_cache_service, redis_cache) = RedisReplicasCacheBackend::new(redis_discovery);
    server.add_service(GenBackgroundService::new(
        format!("Redis Connection Pool Service"),
        Arc::new(redis_cache_service),
    ));
    let cache = Box::leak(Box::new(redis_cache));

    // Proxy service
    let proxy = GrpcProxy {
        dns_discovery,
        service_config,
        cache,
    };
    let mut proxy = pingora_proxy::http_proxy_service(&server.configuration, proxy);

    let mut http_server_options = HttpServerOptions::default();
    http_server_options.h2c = true;
    proxy.app_logic_mut().unwrap().server_options = Some(http_server_options);

    proxy.add_tcp("[::1]:50052");
    server.add_service(proxy);

    // TODO health check service

    //let service =
    //    pingora_core::services::listening::Service::new("GRPC Proxy Service".into(), foob);

    server.run_forever();
}

// =============

struct GrpcProxy {
    dns_discovery: discovery::dns::Handle,
    service_config: service_store::ServiceConfig,
    cache: &'static (dyn Storage + Sync),
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

// .map_err(|cause| {
//         pingora::Error::because(
//             pingora::ErrorType::HTTPStatus(400),
//             "invalid grpc http path",
//             cause,
//         )
//     })?;

struct GrpcMeta {
    service_data: ServiceData,
    method_name: String,
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
struct RequestCtx {
    do_cache: bool,
    grpc_meta: Option<GrpcMeta>,
}

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
            if !service.methods.contains_key(method) {
                log::warn!(
                    "Got request for unknown gRPC method `{}` in service `{}`, passing through. Is the proto descriptors up to date?",
                    method,
                    service_name
                )
            }
        }

        ctx.grpc_meta = Some(GrpcMeta {
            service_data: service_spec.clone(),
            method_name: method.into(),
        });

        if let Some(cache_spec) = ctx
            .grpc_meta
            .as_ref()
            .unwrap()
            .method_spec()
            .and_then(|m| m.cache_spec.as_ref())
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

            if session.retry_buffer_truncated() {
                log::error!("request body above buffer size!");
                return Err(pingora::Error::explain(
                    pingora::ErrorType::HTTPStatus(413),
                    "request body size above buffer size for cache",
                ));
            }
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
        let request_body = session.get_retry_buffer();

        let req_header = session.req_header();
        Ok(CacheKey::default(req_header))
    }

    async fn upstream_peer(
        &self,
        session: &mut Session,
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
            let mut peer = HttpPeer::new_proxy(
                "upstream",
                backend.addr.as_inet().unwrap().ip().into(),
                12345,
                false,
                "",
                BTreeMap::new(),
            );
            peer.options.alpn = ALPN::H2;
            //peer.options.verify_cert = false;
            Ok(Box::new(peer))
        } else {
            Err(pingora::Error::explain(
                pingora::ErrorType::ConnectProxyFailure,
                "no upstream available",
            ))
        }
    }
}
