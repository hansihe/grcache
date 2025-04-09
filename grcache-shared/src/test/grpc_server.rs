use std::{
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use bytes::{BufMut, Bytes, BytesMut};
use h2::{
    server::{self, SendResponse},
    Reason, RecvStream,
};
use http::{request::Parts, HeaderMap, Request};
use tokio::{
    net::{TcpListener, TcpStream},
    spawn,
};

pub struct MockServer {
    pub addr: SocketAddr,
    pub state: Arc<Mutex<State>>,
}

pub struct State {
    got_extra_requests: bool,
    expects: Vec<(String, String, Box<HandleFn>)>,
}

pub type HandleFn = dyn FnOnce(Parts, Bytes) -> (Bytes, HeaderMap) + Send;

impl MockServer {
    pub async fn new() -> Self {
        let state = State {
            got_extra_requests: false,
            expects: Vec::new(),
        };
        let state = Arc::new(Mutex::new(state));

        // Listen on :0, make OS assign port
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        let i_state = state.clone();
        spawn(async move {
            loop {
                let (socket, _addr) = listener.accept().await.unwrap();
                let i_state = i_state.clone();
                spawn(async move {
                    dummy_serve(socket, i_state).await.unwrap();
                });
            }
        });

        MockServer { addr, state }
    }

    pub fn expect(
        &mut self,
        service: &str,
        method: &str,
        handle: impl FnOnce(Parts, Bytes) -> (Bytes, HeaderMap) + Send + 'static,
    ) {
        self.state
            .lock()
            .unwrap()
            .expects
            .push((service.into(), method.into(), Box::new(handle)));
    }

    pub fn finish(self) {
        let lock = self.state.lock().unwrap();
        assert!(!lock.got_extra_requests);
    }
}

async fn dummy_serve(socket: TcpStream, state: Arc<Mutex<State>>) -> Result<(), anyhow::Error> {
    let mut connection = server::handshake(socket).await?;
    println!("H2 connection bound");

    while let Some(result) = connection.accept().await {
        println!("H2 request");
        let (request, respond) = result?;
        let i_state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = dummy_handle_request(request, respond, i_state).await {
                println!("error while handling request: {}", e);
            }
        });
    }

    Ok(())
}

async fn dummy_handle_request(
    mut request: Request<RecvStream>,
    mut respond: SendResponse<Bytes>,
    state: Arc<Mutex<State>>,
) -> Result<(), anyhow::Error> {
    let body = request.body_mut();
    let mut full_body = BytesMut::new();
    while let Some(data) = body.data().await {
        full_body.put(data.unwrap());
    }

    let (parts, _) = request.into_parts();

    {
        let mut guard = state.lock().unwrap();
        if guard.expects.is_empty() {
            respond.send_reset(Reason::REFUSED_STREAM);
            guard.got_extra_requests = true;
        } else {
            let (service, method, handler) = guard.expects.remove(0);
            // TODO send back to main
            assert!(parts.uri.path() == &format!("/{}/{}", service, method));

            let (resp_body, resp_parts) = handler(parts, full_body.freeze());

            let response = http::Response::new(());
            let mut send = respond.send_response(response, false).unwrap();
            send.send_data(resp_body, false).unwrap();
            send.send_trailers(resp_parts).unwrap();
        }
    }

    //let (resp_bytes, resp_trailers) =

    let response = http::Response::new(());
    let mut send = respond.send_response(response, false).unwrap();
    send.send_data(Bytes::from_static(b"\0\0\0\0\0"), false)
        .unwrap();
    send.send_trailers(HeaderMap::new()).unwrap();

    Ok(())
}
