use std::collections::BTreeSet;

use blake2::Digest as _;
use bytes::Bytes;
use http::HeaderMap;
use pingora::http::RequestHeader;

use crate::proxy::Blake2b128;

pub fn hash_vary(hasher: &mut Blake2b128, vary_set: &BTreeSet<String>, headers: &HeaderMap) {
    let mut vary_headers: Vec<_> = headers.get_all("vary").iter().collect();
    vary_headers.sort();

    for vary_header in vary_set.iter() {
        hasher.update(vary_header.as_bytes());
        hasher.update(b"\0");

        let values = headers.get_all(vary_header);
        for value in itertools::Itertools::intersperse(values.iter().map(Some), None) {
            match value {
                None => hasher.update(b", "),
                Some(value) => hasher.update(value.as_bytes()),
            }
        }

        hasher.update(b"\0\0");
    }
}

pub fn hash_body(hasher: &mut Blake2b128, header: &RequestHeader, body: &Bytes) {
    let raw_path = header.raw_path();
    hasher.update(raw_path.len().to_le_bytes());
    hasher.update(raw_path);

    // TODO we can be smarter with how we hash messages to
    // eliminate encoding discrepancies.
    // Protobuf messages do not have a canonical serialization
    // format.
    hasher.update(body.len().to_le_bytes());
    hasher.update(body);
}
