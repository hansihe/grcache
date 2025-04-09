use std::any::Any;

use async_trait::async_trait;
use pingora::cache::{
    key::CompactCacheKey, storage::HandleMiss, trace::SpanHandle, CacheKey, CacheMeta, HitHandler,
    MissHandler, PurgeType, Storage,
};

use crate::cache::GrcacheStorage;

pub struct MockMissHandler {}

#[async_trait]
impl HandleMiss for MockMissHandler {
    async fn write_body(&mut self, data: bytes::Bytes, eof: bool) -> pingora::Result<()> {
        println!("write body: {:?} eof: {}", data, eof);
        Ok(())
    }

    async fn finish(self: Box<Self>) -> pingora::Result<usize> {
        println!("finish cache miss");
        Ok(1000)
    }
}

impl Drop for MockMissHandler {
    fn drop(&mut self) {
        println!("cache miss write abort");
    }
}

pub struct MockStorage {}

impl GrcacheStorage for MockStorage {
    fn as_storage(&self) -> &(dyn Storage + Sync) {
        self
    }
}

#[async_trait]
impl Storage for MockStorage {
    async fn lookup(
        &'static self,
        _key: &CacheKey,
        _trace: &SpanHandle,
    ) -> pingora::Result<Option<(CacheMeta, HitHandler)>> {
        Ok(None)
    }
    async fn get_miss_handler(
        &'static self,
        _key: &CacheKey,
        _meta: &CacheMeta,
        _trace: &SpanHandle,
    ) -> pingora::Result<MissHandler> {
        Ok(Box::new(MockMissHandler {}))
    }
    async fn purge(
        &'static self,
        _key: &CompactCacheKey,
        _purge_type: PurgeType,
        _trace: &SpanHandle,
    ) -> pingora::Result<bool> {
        todo!()
    }
    async fn update_meta(
        &'static self,
        _key: &CacheKey,
        _meta: &CacheMeta,
        _trace: &SpanHandle,
    ) -> pingora::Result<bool> {
        todo!()
    }
    fn as_any(&self) -> &(dyn Any + Send + Sync + 'static) {
        self
    }
}
