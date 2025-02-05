use std::path::PathBuf;

use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube_derive::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_string() -> String {
    "default".into()
}

fn default_grpc_port() -> u16 {
    50051
}

#[derive(CustomResource, Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[kube(
    group = "hansihe.com",
    version = "v1",
    kind = "GrcacheService",
    namespaced,
    selectable = ".spec.cluster"
)]
#[serde(rename_all = "camelCase")]
pub struct GrcacheServiceSpec {
    /// A `grcache` deployment will only pull resources with the same
    /// `cluster` name as itself.
    /// If unset, this defaults to `default`.
    #[serde(default = "default_string")]
    pub cluster: String,

    /// For the case when a single gRPC service is implemented on
    /// multiple upstreams, this option can be used.
    /// When no upstream name is specified in the gRPC request headers,
    /// it defaults to the service spec with no upstream name.
    /// If more than one `GrcacheService` is declared for each
    /// permutation of (service_name, upstream_name), validation
    /// errors will occur.
    /// For 99% of cases this should not be set.
    pub upstream_name: Option<String>,

    /// Declares how upstreams are resolved for this service.
    pub upstream: Upstream,

    /// The name of the gRPC service.
    /// This should match the name of the gRPC service specified in your
    /// proto file, including the full package path.
    /// When making a request to the grcache proxy, `service_name` +
    /// optionally `upstream_name` will decide which upstream server
    /// this request goes to.
    pub service_name: String,

    /// Caching config for RPC methods are declared inline in the proto
    /// files. In order to be able to cache, we need the descriptors for
    /// the protos which contain the service.
    /// If not specified, no caching will be performed, `grcache` will
    /// be in proxy mode only.
    pub descriptor_set_source: Option<DescriptorSetSource>,
}

impl GrcacheService {
    /// Reexport of a function generating the CRD spec to simplify
    /// dependency requirements.
    pub fn crd() -> CustomResourceDefinition {
        <Self as kube::CustomResourceExt>::crd()
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum Upstream {
    /// DNS will be used to resolve the upstreams.
    ///
    /// We are a good DNS service discovery citizen, and will:
    /// * Periodically refresh upstreams based on DNS TTLs.
    /// * Load balance between different upstreams.
    ///
    /// You should be able to point `grcache` at a k8s service (or other
    /// DNS based service discovery) and expect things to mostly just work.
    Dns {
        url: String,
        #[serde(default = "default_grpc_port")]
        port: u16,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum DescriptorSetSource {
    File {
        /// Path to the descriptor set file on the local filesystem.
        path: PathBuf,
    },
    Bucket {
        /// Refers to a bucket name from the main `grcache` config
        /// file.
        name: String,
        /// Hash of the desctiptor set file. Will be used to generate the
        /// object name in the bucket.
        hash: String,
    },
    Inline {
        /// Base64 encoded descriptor set binary.
        /// Subject to API server size limits, use with caution.
        data: String,
    },
}
