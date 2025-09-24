use crate::provider::{Provider, ProviderScope};
use cardinal_config::CardinalConfig;
use cardinal_errors::CardinalError;
use parking_lot::{Mutex, RwLock};
use std::any::{Any, TypeId};
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

pub struct CardinalContext {
    pub config: Arc<CardinalConfig>,
    scopes: RwLock<HashMap<TypeId, ProviderScope>>, // registered scopes for types
    singletons: RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>, // cached singleton instances
    constructing: Mutex<HashSet<TypeId>>,           // basic cycle detection
    factories: RwLock<HashMap<TypeId, Arc<dyn ProviderFactory>>>,
}

impl CardinalContext {
    pub fn new(config: CardinalConfig) -> Self {
        Self {
            config: Arc::new(config),
            scopes: RwLock::new(HashMap::new()),
            singletons: RwLock::new(HashMap::new()),
            constructing: Mutex::new(HashSet::new()),
            factories: RwLock::new(HashMap::new()),
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

    pub fn register_with_factory<T, F, Fut>(&self, scope: ProviderScope, factory: F)
    where
        T: Provider + Send + Sync + 'static,
        F: Fn(&CardinalContext) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, CardinalError>> + Send + 'static,
    {
        let tid = TypeId::of::<T>();
        let factory = Arc::new(TypedFactory::<T, F> {
            inner: factory,
            _marker: PhantomData,
        }) as Arc<dyn ProviderFactory>;

        self.factories.write().insert(tid, factory);
        self.register::<T>(scope);
    }

    pub fn register_singleton_instance<T>(&self, instance: Arc<T>)
    where
        T: Provider + Send + Sync + 'static,
    {
        let tid = TypeId::of::<T>();
        self.register::<T>(ProviderScope::Singleton);
        let erased: Arc<dyn Any + Send + Sync> = instance;
        self.singletons.write().insert(tid, erased);
        self.factories.write().remove(&tid);
    }

    pub fn is_registered<T>(&self) -> bool
    where
        T: Provider + Send + Sync + 'static,
    {
        let tid = TypeId::of::<T>();
        self.scopes.read().contains_key(&tid)
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
                let factory = self.factory_for::<T>();
                let erased: Arc<dyn Any + Send + Sync> = match factory {
                    Some(factory) => factory.create(self).await?,
                    None => Arc::new(T::provide(self).await?) as Arc<dyn Any + Send + Sync>,
                };
                drop(guard);

                // Insert into cache if still absent; another thread might have inserted meanwhile
                {
                    let mut cache = self.singletons.write();
                    cache.entry(tid).or_insert(erased.clone());
                }

                // Return the (possibly newly) cached value
                Arc::downcast::<T>(erased).map_err(|_| {
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
                let factory = self.factory_for::<T>();
                let erased: Arc<dyn Any + Send + Sync> = match factory {
                    Some(factory) => factory.create(self).await?,
                    None => Arc::new(T::provide(self).await?) as Arc<dyn Any + Send + Sync>,
                };
                drop(guard);
                Arc::downcast::<T>(erased).map_err(|_| {
                    CardinalError::InternalError(
                        cardinal_errors::internal::CardinalInternalError::DependencyTypeMismatch,
                    )
                })
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

    fn factory_for<T>(&self) -> Option<Arc<dyn ProviderFactory>>
    where
        T: Provider + Send + Sync + 'static,
    {
        let tid = TypeId::of::<T>();
        self.factories.read().get(&tid).cloned()
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

type ProviderFuture =
    Pin<Box<dyn Future<Output = Result<Arc<dyn Any + Send + Sync>, CardinalError>> + Send>>;

trait ProviderFactory: Send + Sync {
    fn create(&self, ctx: &CardinalContext) -> ProviderFuture;
}

struct TypedFactory<T, F> {
    inner: F,
    _marker: PhantomData<T>,
}

impl<T, F, Fut> ProviderFactory for TypedFactory<T, F>
where
    T: Provider + Send + Sync + 'static,
    F: Fn(&CardinalContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<T, CardinalError>> + Send + 'static,
{
    fn create(&self, ctx: &CardinalContext) -> ProviderFuture {
        let fut = (self.inner)(ctx);
        Box::pin(async move {
            let value = fut.await?;
            Ok(Arc::new(value) as Arc<dyn Any + Send + Sync>)
        })
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
    async fn register_with_factory_preempts_default_provider() {
        #[derive(Debug, Clone, PartialEq, Eq)]
        struct StringProvider(pub String);

        #[async_trait]
        impl Provider for StringProvider {
            async fn provide(_ctx: &CardinalContext) -> Result<Self, CardinalError> {
                Ok(StringProvider("Hello".to_string()))
            }
        }

        let ctx = get_context();
        ctx.register_with_factory::<StringProvider, _, _>(ProviderScope::Singleton, |_ctx| async {
            Ok(StringProvider("Overridden".to_string()))
        });

        assert!(ctx.is_registered::<StringProvider>());
        let a = ctx.get::<StringProvider>().await.unwrap();
        let b = ctx.get::<StringProvider>().await.unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert_eq!(a.0, "Overridden");
    }

    #[tokio::test]
    async fn register_singleton_instance_returns_same_arc() {
        #[derive(Debug)]
        struct Static;

        #[async_trait]
        impl Provider for Static {
            async fn provide(_ctx: &CardinalContext) -> Result<Self, CardinalError> {
                Ok(Static)
            }
        }

        let ctx = get_context();
        let instance = Arc::new(Static);
        ctx.register_singleton_instance::<Static>(instance.clone());

        let a = ctx.get::<Static>().await.unwrap();
        let b = ctx.get::<Static>().await.unwrap();
        assert!(Arc::ptr_eq(&a, &b));
        assert!(Arc::ptr_eq(&a, &instance));
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
