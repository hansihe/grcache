use std::sync::Arc;

use pingora::{apps::HttpServerOptions, services::background::GenBackgroundService};
use pingora_core::{prelude::Opt, server::Server};

pub mod cache;
pub mod discovery;
pub mod proxy;
pub mod service_store;

use cache::redis_replicas::RedisReplicasCacheBackend;
use proxy::GrpcProxy;

#[derive(clap::Parser)]
struct Args {
    /// Path to the `grproxy` config file
    config: String,
}

fn main() {
    env_logger::init();

    let opt = Opt::default();
    //let opt = Opt::parse_args();
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

    let redis_discovery =
        dns_discovery.backends_for_hostname("grcache-dragonfly-nodes".into(), 6379);
    let (redis_cache_service, redis_cache) = RedisReplicasCacheBackend::new(redis_discovery);
    server.add_service(GenBackgroundService::new(
        format!("Redis Connection Pool Service"),
        Arc::new(redis_cache_service),
    ));
    let cache = Box::leak(Box::new(redis_cache));

    // Proxy service
    let proxy = GrpcProxy {
        service_config,
        cache,
    };
    let mut proxy = pingora_proxy::http_proxy_service(&server.configuration, proxy);

    let mut http_server_options = HttpServerOptions::default();
    http_server_options.h2c = true;
    proxy.app_logic_mut().unwrap().server_options = Some(http_server_options);

    proxy.add_tcp("0.0.0.0:50052");
    server.add_service(proxy);

    server.run_forever();
}
