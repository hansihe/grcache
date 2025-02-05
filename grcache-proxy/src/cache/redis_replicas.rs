use std::{any::Any, future::Future, ops::Deref, sync::Arc};

use bb8::Pool;
use bb8_redis::{redis::AsyncCommands, RedisConnectionManager};
use bytes::{BufMut, BytesMut};
use pingora::{
    cache::{
        key::{CacheHashKey, CompactCacheKey},
        storage::{HandleHit, HandleMiss},
        trace::SpanHandle,
        CacheKey, CacheMeta, HitHandler, MissHandler, PurgeType, Storage,
    },
    server::ShutdownWatch,
    services::background::BackgroundService,
};
use pingora_load_balancing::Backend;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::discovery::{self, ServiceBackendsHandle};

pub struct RedisReplicasCacheBackend {
    pools: Arc<RedisPools>,
}

impl RedisReplicasCacheBackend {
    pub fn new(discovery: Arc<ServiceBackendsHandle>) -> (Service, Self) {
        let pools = Arc::new(RedisPools {
            backends: discovery,
            pools: Default::default(),
        });

        let cache_backend = RedisReplicasCacheBackend {
            pools: pools.clone(),
        };
        let service = Service { pools };
        (service, cache_backend)
    }
}

struct RedisCacheHit {
    data: bytes::Bytes,
}

#[async_trait::async_trait]
impl HandleHit for RedisCacheHit {
    async fn read_body(&mut self) -> pingora::Result<Option<bytes::Bytes>> {
        Ok(Some(self.data.clone()))
    }

    /// Finish the current cache hit
    async fn finish(
        self: Box<Self>, // because self is always used as a trait object
        _storage: &'static (dyn Storage + Sync),
        _key: &CacheKey,
        _trace: &SpanHandle,
    ) -> pingora::Result<()> {
        // TODO update cache meta?
        Ok(())
    }

    fn can_seek(&self) -> bool {
        false
    }

    fn as_any(&self) -> &(dyn Any + Send + Sync) {
        self
    }
}

struct RedisCacheMiss {
    pools: Arc<RedisPools>,
    meta: (Vec<u8>, Vec<u8>),
    hash: [u8; 16],
    value: bytes::BytesMut,
}

#[async_trait::async_trait]
impl HandleMiss for RedisCacheMiss {
    async fn write_body(&mut self, data: bytes::Bytes, _eof: bool) -> pingora::Result<()> {
        self.value.put_slice(&data);
        Ok(())
    }

    async fn finish(self: Box<Self>) -> pingora::Result<usize> {
        let miss_data = *self;

        // Fetch connection pool
        let pool = match miss_data.pools.pool_for_hash(&miss_data.hash).await {
            Some(pool) => pool,
            None => {
                // No pool available, do not fetch.
                return Ok(0);
            }
        };

        // Get connection from pool
        let mut conn = match pool.get().await {
            Ok(conn) => conn,
            Err(error) => {
                log::error!("error getting cache connection from pool! {}", error);
                return Ok(0);
            }
        };

        let data_struct = CacheData {
            cache_meta: miss_data.meta,
            data: miss_data.value.into(),
        };

        let mut data = Vec::new();
        data.put_u8(0);
        bincode::serialize_into(&mut data, &data_struct).unwrap();

        let ret: Result<(), _> = conn.set(&miss_data.hash, &data).await;
        if let Err(error) = ret {
            log::error!("failed to set cache value! {}", error);
        }

        // todo
        Ok(data.len())
    }
}

#[derive(Serialize, Deserialize)]
struct CacheData {
    cache_meta: (Vec<u8>, Vec<u8>),
    data: bytes::Bytes,
}

#[async_trait::async_trait]
impl Storage for RedisReplicasCacheBackend {
    async fn lookup(
        &'static self,
        key: &CacheKey,
        _trace: &SpanHandle,
    ) -> pingora::Result<Option<(CacheMeta, HitHandler)>> {
        let hash = key.primary_bin();

        // Fetch connection pool
        let pool = match self.pools.pool_for_hash(&hash[..]).await {
            Some(pool) => pool,
            None => {
                // No pool available, do not fetch.
                return Ok(None);
            }
        };

        // Get connection from pool
        let mut conn = match pool.get().await {
            Ok(conn) => conn,
            Err(error) => {
                log::error!("error getting cache connection from pool! {}", error);
                return Ok(None);
            }
        };

        // Get value from connection
        let data_opt: Option<bytes::Bytes> = conn.get(&hash[..]).await.unwrap_or_else(|error| {
            log::error!("error running redis cache get! {}", error);
            None
        });

        // Handle cache miss
        let data = match data_opt {
            Some(data) => data,
            None => {
                // Cache miss
                return Ok(None);
            }
        };

        // Data version mismatch handling
        let version = data[0];
        if version != 0 {
            log::error!("got bad version tag from cache {} != 0", version);
            return Ok(None);
        }

        match bincode::deserialize::<CacheData>(&data[1..]) {
            Ok(cache_data) => {
                let meta =
                    CacheMeta::deserialize(&cache_data.cache_meta.0, &cache_data.cache_meta.1)?;

                Ok(Some((
                    meta,
                    Box::new(RedisCacheHit {
                        data: cache_data.data,
                    }),
                )))
            }
            Err(error) => {
                log::error!("failed to deserialize cache data! {}", error);
                Ok(None)
            }
        }
    }

    async fn get_miss_handler(
        &'static self,
        key: &CacheKey,
        meta: &CacheMeta,
        _trace: &SpanHandle,
    ) -> pingora::Result<MissHandler> {
        Ok(Box::new(RedisCacheMiss {
            pools: self.pools.clone(),
            meta: meta.serialize()?,
            hash: key.primary_bin(),
            value: bytes::BytesMut::new(),
        }))
    }

    async fn purge(
        &'static self,
        _key: &CompactCacheKey,
        _purge_type: PurgeType,
        _trace: &SpanHandle,
    ) -> pingora::Result<bool> {
        // TODO
        Ok(false)
    }

    async fn update_meta(
        &'static self,
        _key: &CacheKey,
        _meta: &CacheMeta,
        _trace: &SpanHandle,
    ) -> pingora::Result<bool> {
        // TODO
        Ok(false)
    }

    fn as_any(&self) -> &(dyn Any + Send + Sync + 'static) {
        self
    }
}

/// Background service which maintains the memcached connection pool.
pub struct Service {
    pools: Arc<RedisPools>,
}

pub struct RedisPools {
    /// Service discovery handle for `redis` backend.
    backends: Arc<discovery::ServiceBackendsHandle>,
    /// Connection pools maintained by dedicated task.
    pools: papaya::HashMap<Backend, bb8::Pool<bb8_redis::RedisConnectionManager>>,
}

impl RedisPools {
    pub fn select_backend(&self, key: &[u8]) -> Option<Backend> {
        self.backends
            .load_balancer_consistent
            .select_with(key, 255, |backend, healthy| {
                if !healthy {
                    return false;
                }
                self.pools.pin().contains_key(backend)
            })
    }

    pub async fn pool_for_hash(&self, hash: &[u8]) -> Option<Pool<RedisConnectionManager>> {
        if let Some(backend) = self.select_backend(hash) {
            let pool_opt = self.pools.pin().get(&backend).cloned();
            if let Some(pool) = pool_opt {
                Some(pool)
            } else {
                // This can happen if we raced with pool construction/destruction.
                // Should be exceptionally rare, we handle as miss.
                None
            }
        } else {
            None
        }
    }
}

#[async_trait::async_trait]
impl BackgroundService for Service {
    async fn start(&self, _shutdown: ShutdownWatch) {
        let mut changed_handler = self.pools.backends.backends.clone();
        changed_handler.mark_changed();

        // This loops and propagates our `pools` map whenever the set of
        // backends changes from service discovery.
        // If we don't have a pool active for a certain backend, we fail
        // safe, so the time between the backend being added and the
        // pool being created should not be a big issue except for a few
        // potential cache misses.
        loop {
            // If service discovery crashes we want to propagate.
            changed_handler.changed().await.unwrap();
            let backends = changed_handler.borrow().clone();

            self.pools.pools.pin().retain(|k, _v| backends.contains(k));

            for backend in backends.iter() {
                if !self.pools.pools.pin().contains_key(backend) {
                    // We should never really have a UDS here
                    let inet_addr = backend.as_inet().unwrap();

                    let url_str = format!("redis://{}", inet_addr);
                    // Format should always be correct since we construct it
                    // right here.
                    let url: Url = url_str.parse().unwrap();

                    // TODO other options
                    let pool = bb8::Pool::builder()
                        .build(bb8_redis::RedisConnectionManager::new(url).unwrap())
                        .await
                        .unwrap();

                    self.pools.pools.pin().insert(backend.clone(), pool);
                }
            }
        }
    }
}
