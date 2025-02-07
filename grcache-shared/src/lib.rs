use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GrcacheSpecV1 {
    items: Vec<SpecItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecItem {}

pub mod config;

pub mod service;

pub mod field_ref;

pub mod resource_change;

pub mod proto;

pub use protobuf::MessageDyn;

pub mod protos {
    include!(concat!(env!("OUT_DIR"), "/generated/mod.rs"));
}
