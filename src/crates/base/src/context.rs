use crate::provider::{Provider, ProviderScope};
use cardinal_config::CardinalConfig;
use cardinal_errors::CardinalError;
use parking_lot::{Mutex, RwLock};
use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub struct CardinalContext {
    pub config: Arc<CardinalConfig>,
    scopes: RwLock<HashMap<TypeId, ProviderScope>>, // registered scopes for types
    singletons: RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>, // cached singleton instances
    constructing: Mutex<HashSet<TypeId>>,           // basic cycle detection
}

impl CardinalContext {
    pub fn new(config: CardinalConfig) -> Self {
        Self {
            config: Arc::new(config),
            scopes: RwLock::new(HashMap::new()),
            singletons: RwLock::new(HashMap::new()),
            constructing: Mutex::new(HashSet::new()),
        }
    }

    // Register a scope for concrete type T. Overwrites existing scope if re-registered.
    pub fn register<T>(&self, scope: ProviderScope)
    where
        T: Provider + Send + Sync + 'static,
    {
        let tid = TypeId::of::<T>();
        let mut map = self.scopes.write();
        map.insert(tid, scope);
    }

    // Lazily constructs values on first access and caches singletons.
    pub async fn get<T>(&self) -> Result<Arc<T>, CardinalError>
    where
        T: Provider + Send + Sync + 'static,
    {
        let tid = TypeId::of::<T>();

        // Determine scope for T
        let scope = {
            let map = self.scopes.read();
            match map.get(&tid) {
                Some(s) => *s,
                None => {
                    return Err(CardinalError::InternalError(
                        cardinal_errors::internal::CardinalInternalError::ProviderNotRegistered,
                    ))
                }
            }
        };

        match scope {
            ProviderScope::Singleton => {
                // Fast path: already cached
                if let Some(existing) = self.singletons.read().get(&tid).cloned() {
                    return existing
                        .downcast::<T>()
                        .map_err(|_| CardinalError::InternalError(cardinal_errors::internal::CardinalInternalError::DependencyTypeMismatch));
                }

                // Build with cycle detection
                let guard = match self.try_mark_constructing(tid) {
                    Ok(g) => g,
                    Err(e) => return Err(e),
                };
                let value = match T::provide(self).await {
                    Ok(v) => v,
                    Err(e) => return Err(e),
                };
                drop(guard);

                let arc_t = Arc::new(value);
                let erased: Arc<dyn Any + Send + Sync> = arc_t.clone();

                // Insert into cache if still absent; another thread might have inserted meanwhile
                {
                    let mut cache = self.singletons.write();
                    cache.entry(tid).or_insert(erased);
                }

                // Return the (possibly newly) cached value
                let existing = self.singletons.read().get(&tid).cloned().ok_or_else(|| {
                    CardinalError::InternalError(
                        cardinal_errors::internal::CardinalInternalError::ProviderNotBuilt,
                    )
                })?;

                existing.downcast::<T>().map_err(|_| {
                    CardinalError::InternalError(
                        cardinal_errors::internal::CardinalInternalError::DependencyTypeMismatch,
                    )
                })
            }
            ProviderScope::Transient => {
                // Build with cycle detection, do not cache
                let guard = match self.try_mark_constructing(tid) {
                    Ok(g) => g,
                    Err(e) => return Err(e),
                };
                let value = match T::provide(self).await {
                    Ok(v) => v,
                    Err(e) => return Err(e),
                };
                drop(guard);
                Ok(Arc::new(value))
            }
        }
    }

    // Convenience that just calls get<T>(), intended for startup pre-warming.
    pub async fn build_eager<T>(&self) -> Result<Arc<T>, CardinalError>
    where
        T: Provider + Send + Sync + 'static,
    {
        self.get::<T>().await
    }

    fn try_mark_constructing(&self, tid: TypeId) -> Result<ConstructGuard<'_>, CardinalError> {
        let mut set = self.constructing.lock();
        if set.contains(&tid) {
            return Err(CardinalError::InternalError(
                cardinal_errors::internal::CardinalInternalError::DependencyCycleDetected,
            ));
        }
        set.insert(tid);
        Ok(ConstructGuard { ctx: self, tid })
    }

    fn unmark_constructing(&self, tid: TypeId) {
        let mut set = self.constructing.lock();
        set.remove(&tid);
    }
}

// RAII guard for the constructing set, to ensure cleanup on early returns
struct ConstructGuard<'a> {
    ctx: &'a CardinalContext,
    tid: TypeId,
}

impl<'a> Drop for ConstructGuard<'a> {
    fn drop(&mut self) {
        self.ctx.unmark_constructing(self.tid);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cardinal_errors::CardinalError;

    #[derive(Debug)]
    struct Db {
        dsn: String,
    }

    #[derive(Debug)]
    struct Repo {
        db: Arc<Db>,
    }

    #[derive(Debug)]
    struct Service {
        repo: Arc<Repo>,
    }

    #[async_trait]
    impl Provider for Db {
        async fn provide(_ctx: &CardinalContext) -> Result<Self, CardinalError> {
            Ok(Db { dsn: "dsn".into() })
        }
    }

    #[async_trait]
    impl Provider for Repo {
        async fn provide(ctx: &CardinalContext) -> Result<Self, CardinalError> {
            Ok(Repo {
                db: ctx.get::<Db>().await?,
            })
        }
    }

    #[async_trait]
    impl Provider for Service {
        async fn provide(ctx: &CardinalContext) -> Result<Self, CardinalError> {
            Ok(Service {
                repo: ctx.get::<Repo>().await?,
            })
        }
    }

    fn get_context() -> CardinalContext {
        CardinalContext::new(CardinalConfig::default())
    }

    #[tokio::test]
    async fn singleton_reuse_same_arc() {
        let ctx = get_context();
        ctx.register::<Db>(ProviderScope::Singleton);

        let a = ctx.get::<Db>().await.unwrap();
        let b = ctx.get::<Db>().await.unwrap();
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[tokio::test]
    async fn transient_returns_new_arc_each_time() {
        // Use Service/Repo/Db wiring: Service is transient; Repo and Db singletons
        let ctx = get_context();
        ctx.register::<Db>(ProviderScope::Singleton);
        ctx.register::<Repo>(ProviderScope::Singleton);
        ctx.register::<Service>(ProviderScope::Transient);

        let a = ctx.get::<Service>().await.unwrap();
        let b = ctx.get::<Service>().await.unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
    }

    #[tokio::test]
    async fn nested_dependencies_singletons_reused_transient_recreated() {
        let ctx = get_context();
        ctx.register::<Db>(ProviderScope::Singleton);
        ctx.register::<Repo>(ProviderScope::Singleton);
        ctx.register::<Service>(ProviderScope::Transient);

        let s1 = ctx.get::<Service>().await.unwrap();
        let s2 = ctx.get::<Service>().await.unwrap();

        assert!(!Arc::ptr_eq(&s1, &s2));
        assert!(Arc::ptr_eq(&s1.repo, &s2.repo));
        assert!(Arc::ptr_eq(&s1.repo.db, &s2.repo.db));
    }

    struct UnregisteredType;

    #[async_trait]
    impl Provider for UnregisteredType {
        async fn provide(_ctx: &CardinalContext) -> Result<Self, CardinalError> {
            Ok(UnregisteredType)
        }
    }

    #[tokio::test]
    async fn unregistered_type_errors() {
        let ctx = get_context();
        let res = ctx.get::<UnregisteredType>().await;
        assert!(matches!(
            res,
            Err(CardinalError::InternalError(
                cardinal_errors::internal::CardinalInternalError::ProviderNotRegistered
            ))
        ));
    }

    #[derive(Debug)]
    struct A(Arc<B>);
    #[derive(Debug)]
    struct B(Arc<A>);

    #[async_trait]
    impl Provider for A {
        async fn provide(ctx: &CardinalContext) -> Result<Self, CardinalError> {
            Ok(A(ctx.get::<B>().await?))
        }
    }

    #[async_trait]
    impl Provider for B {
        async fn provide(ctx: &CardinalContext) -> Result<Self, CardinalError> {
            Ok(B(ctx.get::<A>().await?))
        }
    }

    #[tokio::test]
    async fn simple_cycle_errors() {
        let ctx = get_context();
        ctx.register::<A>(ProviderScope::Transient);
        ctx.register::<B>(ProviderScope::Transient);

        let res = ctx.get::<A>().await;
        assert!(matches!(
            res,
            Err(CardinalError::InternalError(
                cardinal_errors::internal::CardinalInternalError::DependencyCycleDetected
            ))
        ));
    }
}
