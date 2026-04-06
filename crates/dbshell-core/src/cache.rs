use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::Instant;

use moka::future::Cache;

use crate::operation::DbOperation;
use crate::result::ToolPayload;
use crate::session::CachePolicy;

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub struct CacheKey(pub u64);

impl CacheKey {
    /// Create a cache key from a DbOperation by hashing its debug representation.
    /// This is a pragmatic approach: DbOperation contains types like serde_json::Value
    /// and Vec<f32> that don't implement Hash, so we use the Debug string.
    pub fn from_op(op: &DbOperation) -> Self {
        let mut hasher = DefaultHasher::new();
        format!("{op:?}").hash(&mut hasher);
        CacheKey(hasher.finish())
    }
}

#[derive(Debug, Clone)]
pub struct CachedResult {
    pub payload: ToolPayload,
    pub cached_at: Instant,
}

pub struct CacheLayer {
    session_cache: Option<Cache<CacheKey, CachedResult>>,
}

impl CacheLayer {
    pub fn new(policy: &CachePolicy) -> Self {
        let session_cache = match policy {
            CachePolicy::None => None,
            CachePolicy::SessionScoped => Some(Cache::new(10_000)),
            CachePolicy::Ttl(duration) => Some(
                Cache::builder()
                    .time_to_live(*duration)
                    .max_capacity(10_000)
                    .build(),
            ),
            CachePolicy::Persistent => {
                // Phase 3+: persistent cache backend
                Some(Cache::new(10_000))
            }
        };

        CacheLayer { session_cache }
    }

    pub async fn get(&self, key: &CacheKey) -> Option<CachedResult> {
        let cache = self.session_cache.as_ref()?;
        cache.get(key).await
    }

    pub async fn put(&self, key: CacheKey, result: CachedResult) {
        if let Some(cache) = &self.session_cache {
            cache.insert(key, result).await;
        }
    }

    /// Invalidate all cached entries for the driver+collection/table affected
    /// by this write operation. Since moka doesn't support prefix invalidation,
    /// we invalidate the entire cache on writes for simplicity.
    pub fn invalidate_for_write(&self, _op: &DbOperation) {
        if let Some(cache) = &self.session_cache {
            cache.invalidate_all();
        }
    }
}
