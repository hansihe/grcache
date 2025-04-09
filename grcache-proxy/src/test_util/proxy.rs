use std::{net::SocketAddr, os::fd::AsRawFd, sync::Arc};

use pingora::{
    apps::HttpServerOptions,
    server::{configuration::ServerConf, Fds},
    services::Service as _,
};
use pingora_proxy::http_proxy_service;
use tokio::{
    net::TcpListener,
    sync::{watch, Mutex},
    task::JoinHandle,
};

use crate::{
    proxy::GrpcProxy,
    service_store::{ServiceConfig, ServiceConfigInner},
};

use super::mock_storage::MockStorage;

pub struct ProxyServerTestContext {
    pub ready: watch::Sender<bool>,
    pub shutdown: watch::Sender<bool>,
    pub proxy_handle: JoinHandle<()>,
    pub service_config: ServiceConfig,
    pub listener_addr: SocketAddr,
}

impl ProxyServerTestContext {
    pub async fn shutdown(self) {
        self.shutdown.send(true).unwrap();
        self.proxy_handle.await.unwrap();
    }
}

pub async fn proxy_server() -> ProxyServerTestContext {
    let (s0, r0) = watch::channel(true);

    let service_config = Arc::new(ServiceConfigInner {
        ready: r0,
        services: papaya::HashMap::new(),
    });

    let cache = Box::leak(Box::new(MockStorage {}));

    let proxy = GrpcProxy::new(service_config.clone(), cache);

    let conf = ServerConf::default();
    let mut http_proxy = http_proxy_service(&Arc::new(conf), proxy);

    let mut http_server_options = HttpServerOptions::default();
    http_server_options.h2c = true;
    http_proxy.app_logic_mut().unwrap().server_options = Some(http_server_options);

    http_proxy.add_tcp("127.0.0.1:12345");

    // Create listener with dynamic port and insert into fds.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listener_addr = listener.local_addr().unwrap();
    let mut fds = Fds::new();
    fds.add("127.0.0.1:12345".into(), listener.as_raw_fd());
    let fds = Arc::new(Mutex::new(fds));

    // Do not close the listener.
    std::mem::forget(listener);

    let (s, r) = watch::channel(false);
    let proxy_handle = tokio::spawn(async move {
        http_proxy.start_service(Some(fds), r).await;
    });

    ProxyServerTestContext {
        ready: s0,
        shutdown: s,
        proxy_handle,
        service_config,
        listener_addr,
    }
}
