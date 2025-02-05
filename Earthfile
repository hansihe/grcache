VERSION 0.8
IMPORT github.com/earthly/lib/rust:3.0.1 AS rust

FROM rust:slim-bookworm

build:
    DO rust+INIT --keep_fingerprints=true
    COPY --keep-ts --dir grcache-shared grcache-proxy grcache-cli Cargo.lock Cargo.toml .
    DO rust+CARGO --args="build --release --bin grcache-proxy" --output="release/[^/\.]+"
    DO rust+CARGO --args="build --release --bin grcache-cli" --output="release/[^/\.]+"
    SAVE ARTIFACT target/release/grcache-proxy grcache-proxy
    SAVE ARTIFACT target/release/grcache-cli grcache-cli
