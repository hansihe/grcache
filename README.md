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
