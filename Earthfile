VERSION 0.8
IMPORT github.com/earthly/lib/rust:3.0.1 AS rust

install:
  FROM --platform=linux/amd64 rust:1.84.1-bookworm
  RUN apt-get update -qq
  RUN apt-get install --no-install-recommends -qq autoconf autotools-dev libtool-bin clang cmake bsdmainutils protobuf-compiler
  DO rust+INIT --keep_fingerprints=true

source:
  FROM +install
  COPY --keep-ts --dir grcache-shared grcache-proxy grcache-cli proto Cargo.lock Cargo.toml .

xbuild:
  FROM +source

  ARG TARGETOS
  ARG TARGETARCH
  ARG TARGETVARIANT
  ARG RUSTTARGET

  DO rust+CARGO --target=$RUSTTARGET --args="build --release --bin grcache-proxy --bin grcache-cli" --output="release/[^/\.]+"
  #DO rust+CARGO --target=$target --args="build --release --bin grcache-proxy --bin grcache-cli" --output="release/[^/\.]+"

  SAVE ARTIFACT target/release/grcache-proxy grcache-proxy
  SAVE ARTIFACT target/release/grcache-cli grcache-cli

xbuild-container:
  ARG TARGETPLATFORM
  ARG RUSTTARGET
  FROM --platform=$TARGETPLATFORM debian:bookworm-slim
  WORKDIR /app
  COPY (+xbuild/grcache-proxy --RUSTTARGET=$RUSTTARGET) .
  COPY (+xbuild/grcache-cli --RUSTTARGET=$RUSTTARGET) .
  ENTRYPOINT ["/app/grcache-proxy"]
  SAVE IMAGE --push ghcr.io/hansihe/grcache-proxy:latest

build-container-all:
    BUILD --platform=linux/amd64 +xbuild-container --RUSTTARGET=x86_64-unknown-linux-gnu
    BUILD --platform=linux/arm64 +xbuild-container --RUSTTARGET=aarch64-unknown-linux-gnu
