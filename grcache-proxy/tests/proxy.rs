use std::{any::Any, sync::Arc, time::Duration};

use async_trait::async_trait;
use bytes::BufMut;
use grcache_shared::service::{qualified_service::QualifiedService, ServiceSpec};
use h2::{client, RecvStream};
use http::{HeaderMap, Method, Request, Response, StatusCode};
use mockall::mock;
use pingora::{
    apps::HttpServerOptions,
    cache::Storage,
    server::{configuration::ServerConf, Fds},
    services::Service,
};
use pingora_proxy::http_proxy_service;

use grcache_proxy::{
    discovery::ServiceBackendsHandle,
    proxy::GrpcProxy,
    service_store::{ServiceConfig, ServiceConfigInner, ServiceData},
};

use pingora::cache::{
    key::CompactCacheKey, trace::SpanHandle, CacheKey, CacheMeta, HitHandler, MissHandler,
    PurgeType,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::watch,
    task::JoinHandle,
};

struct MockStorage {}

#[async_trait]
impl Storage for MockStorage {
    async fn lookup(
        &'static self,
        key: &CacheKey,
        trace: &SpanHandle,
    ) -> pingora::Result<Option<(CacheMeta, HitHandler)>> {
        todo!()
    }
    async fn get_miss_handler(
        &'static self,
        key: &CacheKey,
        meta: &CacheMeta,
        trace: &SpanHandle,
    ) -> pingora::Result<MissHandler> {
        todo!()
    }
    async fn purge(
        &'static self,
        key: &CompactCacheKey,
        purge_type: PurgeType,
        trace: &SpanHandle,
    ) -> pingora::Result<bool> {
        todo!()
    }
    async fn update_meta(
        &'static self,
        key: &CacheKey,
        meta: &CacheMeta,
        trace: &SpanHandle,
    ) -> pingora::Result<bool> {
        todo!()
    }
    fn as_any(&self) -> &(dyn Any + Send + Sync + 'static) {
        self
    }
}

struct ProxyServerTestContext {
    ready: watch::Sender<bool>,
    shutdown: watch::Sender<bool>,
    proxy_handle: JoinHandle<()>,
    service_config: ServiceConfig,
}

impl ProxyServerTestContext {
    async fn shutdown(self) {
        self.shutdown.send(true).unwrap();
        self.proxy_handle.await.unwrap();
    }
}

fn proxy_server() -> ProxyServerTestContext {
    let (s0, r0) = watch::channel(true);

    let service_config = Arc::new(ServiceConfigInner {
        ready: r0,
        services: papaya::HashMap::new(),
    });

    let cache = Box::leak(Box::new(MockStorage {}));

    let proxy = GrpcProxy {
        service_config: service_config.clone(),
        cache,
    };

    let conf = ServerConf::default();
    let mut http_proxy = http_proxy_service(&Arc::new(conf), proxy);

    let mut http_server_options = HttpServerOptions::default();
    http_server_options.h2c = true;
    http_proxy.app_logic_mut().unwrap().server_options = Some(http_server_options);

    http_proxy.add_tcp("[::1]:12345");

    let (s, r) = watch::channel(false);
    let proxy_handle = tokio::spawn(async move {
        http_proxy.start_service(None, r).await;
    });

    ProxyServerTestContext {
        ready: s0,
        shutdown: s,
        proxy_handle,
        service_config,
    }
}

async fn grpc_request(service: &str, method: &str, message: &[u8]) -> Response<RecvStream> {
    let tcp = TcpStream::connect("localhost:12345").await.unwrap();
    let (h2, connection) = client::handshake(tcp).await.unwrap();
    tokio::spawn(async move {
        connection.await.unwrap();
    });

    let mut h2 = h2.ready().await.unwrap();
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/{}/{}", service, method))
        .header("content-type", "application/grpc")
        .body(())
        .unwrap();
    let (response, mut send_stream) = h2.send_request(request, false).unwrap();

    let mut data = bytes::BytesMut::new();
    data.put_u8(0);
    data.put_u32(message.len() as u32);
    data.put_slice(message);
    send_stream.send_data(data.freeze(), true).unwrap();

    response.await.unwrap()
}

#[tokio::test]
async fn full_request() {
    let server_test_ctx = proxy_server();

    // Unknown service and method returns not found (404).
    let (head, _body) = grpc_request("package.Service", "Method", b"")
        .await
        .into_parts();
    assert!(head.status == StatusCode::NOT_FOUND);

    let (backends_handle, _ready_sender, backends_sender) = ServiceBackendsHandle::new_test();

    server_test_ctx.service_config.services.pin().insert(
        "package.Service".into(),
        ServiceData {
            generation: 10,
            service_spec: Some(Arc::new(ServiceSpec::build_passthrough(
                &QualifiedService::parse("package.Service").unwrap(),
            ))),
            load_balancer: Some(backends_handle.clone()),
        },
    );

    // No backends available returns bad gateway (502)
    let (head, _body) = grpc_request("package.Service", "Method", b"")
        .await
        .into_parts();
    assert!(head.status == StatusCode::BAD_GATEWAY);

    server_test_ctx.shutdown().await;

    todo!()
}
