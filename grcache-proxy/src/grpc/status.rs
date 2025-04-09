use std::{num::ParseIntError, str::Utf8Error};

use http::HeaderMap;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("no status header in trailers")]
    NoStatusHeader,
    #[error("status header value was invalid utf8")]
    InvalidStatusHeaderText(#[from] Utf8Error),
    #[error("status header value was invalid number")]
    InvalidStatusHeaderInt(#[from] ParseIntError),
}

pub struct GrpcStatus {
    code: u8,
}

impl GrpcStatus {
    pub fn parse(trailers: &HeaderMap) -> Result<Self, ParseError> {
        let status_value_bin = trailers
            .get("grpc-status")
            .ok_or_else(|| ParseError::NoStatusHeader)?;
        let status_value_str = std::str::from_utf8(status_value_bin.as_bytes())?;
        let status_code = status_value_str.parse::<u8>()?;

        Ok(GrpcStatus { code: status_code })
    }

    pub fn is_ok(&self) -> bool {
        self.code == 0
    }
}
