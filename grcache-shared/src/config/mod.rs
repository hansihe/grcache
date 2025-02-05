use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub mod context;
pub mod crd;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Configuration for kubernetes integration.
    /// Disabled by default.
    kubernetes: Option<KubernetesConfig>,

    /// Configuration for any number of S3 buckets.
    buckets: HashMap<String, BucketConfig>,

    /// Inline declaration for a number of protobuf descriptor
    /// sets. When running in a kubernetes setting, the kubernetes
    /// integration would likely be used instead.
    protobuf_descriptor_sets: Vec<ProtobufDescriptorSetConfig>,

    /// Inline declaration for a number of eviction events.
    /// When running in a kubernetes setting, the kubernetes
    /// integration would likely be used instead.
    eviction_events: Vec<EvictionEventConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KubernetesConfig {
    /// If true, will enable kubernetes integration.
    /// This will:
    /// * Load protobuf descriptor sets from instances of the
    ///   `GrcacheProtoDescriptorSet` CRD.
    /// * Load model from instances of the
    ///   `GrcacheModel` CRD.
    /// * Disallow inline declarations of the two above.
    enable: bool,

    /// This setting makes it possible to use multiple grcache
    /// clusters within a single k8s namespace. grcache instances
    /// will only pick up k8s resources with this `clusterName`
    /// set on them.
    cluster_name: Option<String>,

    /// Internal option, here be dragons.
    /// Will allow inline resource declarations while k8s integration
    /// is active.
    #[cfg(feature = "internal")]
    internal_allow_inline_resources: bool,
}

/// Configuration for a single S3 bucket.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "source", rename_all = "camelCase")]
pub enum BucketConfig {
    /// Fetch configuration for the bucket from environment
    /// variables.
    ///
    /// Will read the following env vars:
    /// * "{prefix}NAME"
    /// * "{prefix}ENDPOINT"
    /// * "{prefix}KEY_ID"
    /// * "{prefix}KEY_SECRET"
    Env {
        /// Will be prepended as a prefix to the environment variable
        /// names.
        prefix: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "source", rename_all = "camelCase")]
pub enum ProtobufDescriptorSetConfig {
    File {
        /// Local path to a binary proto descriptor file.
        path: String,
    },
    Bucket {
        /// Name of the bucket from the `buckets` config key.
        name: String,
        /// SHA-1 hash of the file in the s3 bucket.
        /// Expected to be a hex string of length 32.
        ///
        /// Will be lowercased before being looked up from the
        /// bucket.
        /// The hash is used directly as the object name in the
        /// bucket.
        ///
        /// After downloading, the SHA-1 hash will be computed
        /// and verified.
        hash: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum EvictionEventConfig {}
