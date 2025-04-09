use std::net::SocketAddr;

use bytes::BufMut as _;
use h2::{client, RecvStream};
use http::{Method, Request, Response};
use tokio::net::TcpStream;

pub async fn grpc_request(
    addr: &SocketAddr,
    service: &str,
    method: &str,
    message: &[u8],
) -> Response<RecvStream> {
    let tcp = TcpStream::connect(addr).await.unwrap();
    let (h2, connection) = client::handshake(tcp).await.unwrap();
    tokio::spawn(async move {
        connection.await.unwrap();
    });

    let mut h2 = h2.ready().await.unwrap();
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/{}/{}", service, method))
        .header("content-type", "application/grpc")
        .header("host", "localhost")
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
