use std::any::Any;

use pingora::cache::{
    key::CompactCacheKey, trace::SpanHandle, CacheKey, CacheMeta, HitHandler, MissHandler,
    PurgeType, Storage,
};
use tinyufo::TinyUfo;

pub struct LocalCacheBackend {
    cache: TinyUfo<(), ()>,
}

impl LocalCacheBackend {
    pub fn new() -> Self {
        LocalCacheBackend {
            cache: TinyUfo::new(5, 5),
        }
    }
}

#[async_trait::async_trait]
impl Storage for LocalCacheBackend {
    async fn lookup(
        &'static self,
        key: &CacheKey,
        trace: &SpanHandle,
    ) -> pingora::Result<Option<(CacheMeta, HitHandler)>> {
        todo!()
    }

    async fn get_miss_handler(
        &'static self,
        key: &CacheKey,
        meta: &CacheMeta,
        trace: &SpanHandle,
    ) -> pingora::Result<MissHandler> {
        todo!()
    }

    async fn purge(
        &'static self,
        key: &CompactCacheKey,
        purge_type: PurgeType,
        trace: &SpanHandle,
    ) -> pingora::Result<bool> {
        todo!()
    }

    async fn update_meta(
        &'static self,
        key: &CacheKey,
        meta: &CacheMeta,
        trace: &SpanHandle,
    ) -> pingora::Result<bool> {
        todo!()
    }

    fn as_any(&self) -> &(dyn Any + Send + Sync + 'static) {
        self
    }
}
