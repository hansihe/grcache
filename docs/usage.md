# Usage example

This goes through how you would set up a basic `grcache` deployment.

We start by setting up a basic deployment which can proxy requests, then proceed to enable caching and more in later steps.

## 1. Basic setup

To get `grproxy` set up and proxying requests, you need the following:
* The `GrcacheService` CRD installed on your Kubernetes cluster
* A `grcache-proxy` deployment on your Kubernetes cluster
* `GrcacheService` k8s objects mapping `gRPC` services to their upstreams

### `GrcacheService` CRD
`grcache-proxy` uses `GrcacheService` objects to declare `gRPC` services and their upstreams. This CRD needs to be installed on your cluster.

```bash
kubectl apply -f grcache-cli/spec/grcache_service_crd.yaml
```

### `grcache-proxy` deployment
At its most basic, this is just a `Kubernetes` deployment with N replicas.

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: grcache-proxy
spec:
  replicas: 1
  selector:
    matchLabels:
      app: grcache
      component: proxy
  template:
    metadata:
      labels:
        app: grcache
        component: proxy
    spec:
      containers:
      - name: grcache-proxy
        image: ghcr.io/hansihe/grcache-proxy:latest
        env:
        - name: RUST_LOG
          value: info
        ports:
        - containerPort: 50052
          name: grpc
```

You probably also want a `Service` for your `gRPC` client to connect to:
```yaml
apiVersion: v1
kind: Service
metadata:
  name: grcache-proxy
  labels:
    app: grcache
    component: proxy
spec:
  type: ClusterIP
  ports:
  - port: 50052
    targetPort: grpc
    protocol: TCP
    name: grpc
  selector:
    app: grcache
    component: proxy
```

### `GrcacheService` objects
Your `grcache` deployment should be running and be ready to accept connections, but it is not aware of any `gRPC` services yet.

You declare `gRPC` services using `GrcacheService` objects which can be applied through your normal deployment pipeline.

```yaml
apiVersion: hansihe.com/v1
kind: GrcacheService
metadata:
  name: example-grpc-service
spec:
  # The fully qualified name of your `gRPC` service
  serviceName: example.ExampleService
  # This declares how `grcache` will find upstreams to proxy
  # requests to. Here we declare a `dns` upstream.
  # DNS service discovery is a good citizen. It respects TTLs,
  # and should work well on Kubernetes.
  upstream:
    dns:
      url: auth-rpc
```

`GrcacheService` objects are live loaded/reloaded/unloaded by `grcache-proxy`. `grcache-proxy` does not need to be restarted.

### Send requests

You can now point your `gRPC` clients at `grcache-proxy` and start sending `gRPC` requests to the `example.ExampleService` service.

Nothing will be cached yet, requests are just passed through.

## 2. Enabling caching with HTTP headers

A basic way to tell `grcache` to start proxying requests is with HTTP headers.

In your `gRPC` client, set the `grpc-cache-control` header to enable caching for a `gRPC` request.

```
grpc-cache-control: max-age=300   # allow response to be cached for 5 minutes
```

## 3. Caching into `redis`

By default `grcache` uses an in memory cache backend. This has the following disadvantages.

* Different replicas of `grcache-proxy` do not share a cache store
* If a `grcache-proxy` instance is killed, we lose all cached values

As such, this mode is not intended for production use.

### `redis_cluster` cache backend

The `redis_cluster` cache backend will shard the cache across N independent `redis` instances.

* We use service discovery to discover redis instances
* These redis instances are intended to be running in cache mode, data not persisted
* We use concistent hashing, a key will deterministically be assigned into a single redis instance
* If a redis instance gets killed, we only lose a small fraction of the cache at a time

Add the following to the `grcache` config:

```yaml
cacheBackend:
  redisReplicas:
    hostname: grcache-redis-service
```

## 4. Providing `grcache-proxy` with `protobuf` descriptors

So far, `grcache` is not aware of the structure of the requests it is caching. This comes with a few disadvantages:
* Since `protobuf` does not have a canonical form, we may end up with duplicate cache entries.
* We cannot specify cache endpoints per `gRPC` method.

Both of these can be addressed by providing `gRPC` with proto descriptors.

If you are using buf, build descriptors for your protos like this:
```bash
buf build -o proto_descriptors.binpb --exclude-source-info
```

We can then augment our `GrcacheService` object with our descriptors:

```yaml
apiVersion: hansihe.com/v1
kind: GrcacheService
metadata:
  name: example-grpc-service
spec:
  # Same as before
  serviceName: example.ExampleService
  upstream:
    dns:
      url: auth-rpc

  # We provide `grcache` with our `proto` descriptors.
  descriptorSetSource:
    # Provide descriptor inline in this example.
    # In a production environment, you would likely not want to use `inline`
    # as you should not make the K8S control plane deal with large values,
    # it can cause performance issues and you can hit object size limits
    # with even moderately sized protobuf descriptors.
    inline:
      data: <base64 encoded contents of `proto_descriptors.binpb`>

    # We can also provide descriptors as:
    # * A local file. These need to be present in the filesystem of
    #   `grcache-proxy` instances
    # * A S3 bucket object
    # * A HTTP URL
```

Again, `grcache` will live reload `GrcacheService`s when you change them in k8s.

## 5. Declare options inline in your `proto` files

Options for `grcache` can be declared inline in your proto files. `grcache` will pick these up when loading descriptors.

```proto
import "grcache/options.proto";

service ExampleService {
  rpc GetData (GetDataRequest) returns (GetDataResponse) {
    option (grcache) = {
      cache_ttl: 3600 // 1 hour
    };
  }
}
```

Options passed in request headers will always override defaults.

## 6. Advanced features

TODO most of these are not implemented yet, but are low effort to implement.

Advanced features which are not covered here include:
* "Sticky load balancing", `grcache` can balance traffic across your upstreams deterministically according to a customizable set of fields in your request messages. This would make in process caching in your gRPC servers more effective.
* Explicit cache evictions. You can evict entities you know have changed from cache. NOTE: It is very difficult to provide rigid cache coherence guarantees here.
* Cache validation. If validating the freshness of a response is cheaper than rebuilding it, `grcache` can be configured to make separate cache validation requests to the upstream.
* Advanced upstream routing. Route requests to different upstreams based on conditions on request fields, by percentage, or mirror traffic. Enables efficient canary deployments, red-green, other deployment strategies.
