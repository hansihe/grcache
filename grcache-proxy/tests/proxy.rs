use grcache_shared::test::{grpc_client::grpc_request, grpc_server::MockServer};
use http::{HeaderMap, StatusCode};

use grcache_proxy::test_util::ProxyTest;

#[tokio::test]
async fn request_without_service_match() {
    let proxy_test = ProxyTest::new().await;

    // Unknown service and method returns not found (404).
    let (head, _body) = grpc_request(&proxy_test.addr(), "package.Service", "Method", b"")
        .await
        .into_parts();
    assert!(head.status == StatusCode::NOT_FOUND);

    proxy_test.shutdown().await;
}

#[tokio::test]
async fn request_with_no_backends() {
    let mut proxy_test = ProxyTest::new().await;
    let _backends_test = proxy_test.add_service_passthrough("package.Service");

    // No backends available returns bad gateway (502)
    let (head, _body) = grpc_request(&proxy_test.addr(), "package.Service", "Method", b"")
        .await
        .into_parts();
    assert!(head.status == StatusCode::BAD_GATEWAY);

    proxy_test.shutdown().await;
}

#[tokio::test]
async fn request_with_passthrough_backend() {
    let mut mock_server = MockServer::new().await;
    mock_server.expect("package.Service", "Method", |_parts, body| {
        assert!(&*body == b"\0\0\0\0\0");
        (bytes::Bytes::from_static(b"\0\0\0\0\0"), HeaderMap::new())
    });

    let mut proxy_test = ProxyTest::new().await;
    let mut backends_test = proxy_test.add_service_passthrough("package.Service");
    backends_test
        .set_single_backend_addr(mock_server.addr)
        .await;

    let (head, _body) = grpc_request(&proxy_test.addr(), "package.Service", "Method", b"")
        .await
        .into_parts();
    assert!(head.status.is_success());

    proxy_test.shutdown().await;
    mock_server.finish();
}

#[tokio::test]
async fn request_with_service_full_backend() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();

    let mut mock_server = MockServer::new().await;
    mock_server.expect("example.TestService", "GetData", |_parts, body| {
        assert!(&*body == b"\0\0\0\0\0");
        (bytes::Bytes::from_static(b"\0\0\0\0\0"), HeaderMap::new())
    });

    let mut proxy_test = ProxyTest::new().await;
    let mut backends_test = proxy_test
        .add_service("example.TestService", "tests/data/proto_descriptors.binpb")
        .await;
    backends_test
        .set_single_backend_addr(mock_server.addr)
        .await;

    let (head, _body) = grpc_request(&proxy_test.addr(), "example.TestService", "GetData", b"")
        .await
        .into_parts();
    println!("{:?}", head);
    assert!(head.status.is_success());

    proxy_test.shutdown().await;
    mock_server.finish();

    todo!();
}
