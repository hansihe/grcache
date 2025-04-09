use pingora::cache::Storage;

//pub mod local;
pub mod redis_cluster;
pub mod redis_replicas;

pub trait GrcacheStorage: Sync {
    fn as_storage(&self) -> &(dyn Storage + Sync);
}
