# Grcache
Grcache is a gRPC caching solution built on Cloudflares `pingora`.

Implementing caching in a generic way for gRPC comes with a variety of
challenges which this project attempts to solve.

[Full usage walkthrough](docs/usage.md)

* Proxy based on `cloudflare`s `pingora` framework.
* Cache `gRPC` request according to policies declared in your `proto` files.
* Declare upstreams using the `GrcacheService` CRD. Services are live reloaded.
* Flexible service discovery.
  * `DNS` - Discover upstreams using DNS. Respects TTLs, balances load.
  * `kube` - NOT IMPLEMENTED. Discover upstreams directly from k8s `service`s.
* Flexible caching backends.
  * `memory` - Cache requests in memory in `grcache-proxy` instances.
  * `redis_replicas` - Use a fleet of independent `redis` replicas to cache requests. TODO explainer on why we do it this way.

At it's core, grcache is a reverse proxy (like nginx) with a set of
special features enabling it to cache gRPC calls in a customizable way.

Caching options are declared inline in your gRPC `protobuf` service files
(see `proto/grcache/options.proto`). The `FileDescriptorSet` which `protoc`
spits out are then loaded into `grcache`, which enables it to cache and
handle requests according to policies you declare. Caching options can
also be overridden on a per request basis if needed.

## Components

### Runtime
A deployment consists of 2 components:
* `grcache-proxy` - Implements the caching proxy itself.
* `grcache-keeper` - Does various bookkeeping tasks, including evictions.

`grcache-keeper` is strictly speaking not required, but without it only
basic functionality like caching with TTL will be available. Evictions
require `grcache-keeper`.

### CLI
`grcache-cli` contains various CLI actions which can be helpful for
developing and deploying `grcache`.

Major features include:
* Validate configs locally. Useful for local development.
* Validate the state of a cluster.
* Validate the state of a cluster given a set of changes. Useful for
  validations in deployment hooks.
* Upload descriptor sets (proto definitions) to s3 bucket in preparation
  for deployment.

## Config
In order to start pods in a `grcache` deployment, one file is needed:
* `grcache.config.yaml` - Contains general operational configuration like
  how to connect to cache backends, kafka brokers (if applicable), etc.
  * Required in order to start.

---

TODO stuff below is not updated

## Service and model data
`grcache` will start without these being specified, but will not do anything.
* `grcache.models.yaml` - Contains definition of things like eviction
  events, upstreams, etc. Multiple `grcache.models.yaml` are supported
  for a single `grcache` deployment.
  * Can also be defined as `GrcacheModels` custom resources in k8s.
  * Zero or more can be provided.
  * Can be reloaded (depending on data backend used)
* `grcache.descriptors.yaml` - Automatically generated grpc service and method
  cache configuration. This is meant to be automatically generated from
  proto files using `grcache-cli`.
  * Zero or more can be provided.
  * Can be reloaded (depending on data backend used)

## Data backends
* `file` - Reads data from plaintext files
* `k8s` - Reads data from kubernetes resources
