use std::{fs::File, io::Read, sync::Arc};

use clap::Parser;
use grcache_shared::{
    config::{CacheBackend, ConfigFile},
    health::Health,
};
use hickory_resolver::TokioAsyncResolver;
use pingora::{apps::HttpServerOptions, services::background::GenBackgroundService};
use pingora_core::{prelude::Opt, server::Server};

pub mod cache;
pub mod discovery;
pub mod grpc;
pub mod proxy;
pub mod service_store;
pub mod tracing;

use cache::{redis_replicas::RedisReplicasCacheBackend, GrcacheStorage};
use proxy::GrpcProxy;

#[derive(clap::Parser)]
struct Args {
    /// Path to the `grproxy` config file
    config: String,

    /// Subcommand to execute
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    Proxy {},
}

fn main() {
    env_logger::init();

    let args = Args::parse();

    let config: ConfigFile = {
        let mut config_file = File::open(&args.config).expect("could not open config file");
        let mut config_buf = Vec::new();
        config_file.read_to_end(&mut config_buf).unwrap();
        let config_str = String::from_utf8(config_buf).unwrap();
        toml::from_str(&config_str).unwrap()
    };

    let opt = Opt::default();
    //let opt = Opt::parse_args();
    // Daemonization is not supported due to how we create resources before
    // calling `run_forever`.
    assert!(!opt.daemon, "daemonization not supported");

    let mut server = Server::new(Some(opt)).unwrap();
    server.bootstrap();

    // Health check subsystem
    let (health, health_root) = Health::new();

    let dns_resolver = Arc::new(
        TokioAsyncResolver::tokio_from_system_conf().expect("failed to create DNS resolver"),
    );

    // DNS discovery background service
    let (dns_service, dns_discovery) = discovery::dns::Service::new(dns_resolver.clone());
    server.add_service(GenBackgroundService::new(
        format!("DNS Discovery Service"),
        Arc::new(dns_service),
    ));

    // For now we only support the case where the k8s backend is
    // enabled.
    assert!(
        config.kubernetes.enable,
        "disabling k8s backend currently not supported"
    );

    // Kubernetes config loader
    let (k8s_config_service, service_config) =
        service_store::K8SConfigService::new(dns_discovery.clone(), health.add(true));
    server.add_service(GenBackgroundService::new(
        format!("Kubernetes Config Service"),
        Arc::new(k8s_config_service),
    ));

    // Initialize cache backend depending on config.
    let cache: Box<dyn GrcacheStorage + Sync + 'static> = match config.cache_backend {
        CacheBackend::RedisReplicas { hostname, port } => {
            let redis_discovery = dns_discovery.backends_for_hostname(hostname, port);
            let (redis_cache_service, redis_cache) =
                RedisReplicasCacheBackend::new(redis_discovery, health.add(true));
            server.add_service(GenBackgroundService::new(
                format!("Redis Connection Pool Service"),
                Arc::new(redis_cache_service),
            ));
            Box::new(redis_cache)
        }
    };
    let cache = Box::leak(cache);

    // Proxy service
    let proxy = GrpcProxy::new(service_config, cache);
    let mut proxy = pingora_proxy::http_proxy_service(&server.configuration, proxy);

    let mut http_server_options = HttpServerOptions::default();
    http_server_options.h2c = true;
    proxy.app_logic_mut().unwrap().server_options = Some(http_server_options);

    proxy.add_tcp("0.0.0.0:50052");
    server.add_service(proxy);

    // Indicate readiness and loop forever
    health_root.ready();
    server.run_forever();
}
