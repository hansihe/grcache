use std::collections::BTreeSet;

use http::{header::ToStrError, HeaderMap};
use phf::phf_set;

/// Takes a HeaderMap with, pulls, cleans and splits `vary` headers,
/// and outputs a `BTreeSet`.
pub fn make_vary_headers_set(headers: &HeaderMap) -> Result<BTreeSet<String>, pingora::BError> {
    let vary_set: Result<BTreeSet<String>, ToStrError> = headers
        .get_all("vary")
        .iter()
        .map(|v| v.to_str())
        .flat_map(|v| v.map(|i| i.split(",")).into_iter().flatten().map(Ok))
        .map(|v| v.map(|v| v.trim().to_string()))
        .collect();

    vary_set.map_err(|e| {
        pingora::Error::because(
            pingora::ErrorType::InvalidHTTPHeader,
            "invalid vary header",
            e,
        )
    })
}

static CORE_HEADERS: phf::Set<&'static str> = phf_set! {
    "host",
    "content-type",
    "te",
    "vary",
    "user-agent"
};

/// Returns all headers which should be stripped.
pub fn find_strip_headers(
    req_headers: &HeaderMap,
    // Headers which are specified as vary by the requester.
    // These are taken into account in the cache key.
    vary_set: &BTreeSet<String>,
    // Headers which are propagated to upstream but not taken
    // into account by the cache key.
    propagation_set: &BTreeSet<String>,
) -> Vec<String> {
    req_headers
        .keys()
        .map(|key| key.as_str().to_owned())
        .filter(|key_str| {
            // Keep header if OR:
            // * It's part of the core headers
            // * It's specified as vary
            // * It's a propagation header
            let keep_header = CORE_HEADERS.contains(key_str)
                || vary_set.contains(key_str)
                || propagation_set.contains(key_str);

            !keep_header
        })
        .collect()
}
