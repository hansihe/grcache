use pingora::http::RequestHeader;

use crate::proxy::parse_grpc_path;

pub enum ProxyLogicState {
    /// When created, we start in the `Init` state.
    /// When we receive request information, we transition into
    /// the `DoCache` or `Bypass` states.
    Init,

    DoCache(CacheState),

    Bypass,
}

struct CacheState {}

impl ProxyLogicState {
    pub fn init() -> Self {
        Self::Init
    }

    pub fn process_req_headers(self, header: &RequestHeader) -> pingora::Result<Self> {
        let (service_name, method) = parse_grpc_path(header.raw_path()).map_err(|cause| {
            pingora::Error::because(
                pingora::ErrorType::HTTPStatus(400),
                "invalid grpc http path",
                cause,
            )
        })?;

        todo!()
    }
}
