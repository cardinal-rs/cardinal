use cardinal_base::context::CardinalContext;
use cardinal_base::destinations::container::DestinationContainer;
use cardinal_base::provider::{Provider, ProviderScope};
use cardinal_config::{load_config, CardinalConfig};
use cardinal_errors::internal::CardinalInternalError;
use cardinal_errors::CardinalError;
use cardinal_plugins::container::PluginContainer;
use cardinal_proxy::CardinalProxy;
use pingora::prelude::Server;
use pingora::proxy::http_proxy_service;
use std::sync::Arc;

pub struct Cardinal {
    context: Arc<CardinalContext>,
}

impl Cardinal {
    pub fn builder(config: CardinalConfig) -> CardinalBuilder {
        CardinalBuilder::new(config)
    }

    pub fn from_paths(config_paths: &[String]) -> Result<Self, CardinalError> {
        Ok(CardinalBuilder::from_paths(config_paths)?.build())
    }

    pub fn new(config: CardinalConfig) -> Self {
        CardinalBuilder::new(config).build()
    }

    pub fn context(&self) -> Arc<CardinalContext> {
        self.context.clone()
    }

    pub fn run(&self) -> Result<(), CardinalError> {
        let mut server = Server::new(None).map_err(|e| {
            CardinalError::InternalError(CardinalInternalError::FailedToInitiateServer(
                e.to_string(),
            ))
        })?;
        server.bootstrap();

        let proxy = CardinalProxy::builder(self.context.clone()).build();
        let mut proxy_service = http_proxy_service(&server.configuration, proxy);

        let server_addr = self.context.config.server.address.clone();

        proxy_service.add_tcp(&server_addr);

        tracing::info!(addr = %server_addr, "Listening on address");

        server.add_service(proxy_service);
        server.run_forever();
    }
}

pub struct CardinalBuilder {
    context: Arc<CardinalContext>,
    auto_register_defaults: bool,
}

impl CardinalBuilder {
    pub fn new(config: CardinalConfig) -> Self {
        let context = Arc::new(CardinalContext::new(config));
        Self {
            context,
            auto_register_defaults: true,
        }
    }

    pub fn new_empty(config: CardinalConfig) -> Self {
        let context = Arc::new(CardinalContext::new(config));
        Self {
            context,
            auto_register_defaults: false,
        }
    }

    pub fn from_paths(config_paths: &[String]) -> Result<Self, CardinalError> {
        let config = load_config(config_paths)?;
        Ok(Self::new(config))
    }

    pub fn context(&self) -> Arc<CardinalContext> {
        self.context.clone()
    }

    pub fn register_provider<T>(self, scope: ProviderScope) -> Self
    where
        T: Provider + Send + Sync + 'static,
    {
        self.context.register::<T>(scope);
        self
    }
    pub fn register_provider_with_factory<T, F>(self, scope: ProviderScope, factory: F) -> Self
    where
        T: Provider + Send + Sync + 'static,
        F: Fn(Arc<CardinalContext>) -> Result<T, CardinalError> + Send + Sync + 'static,
    {
        let ctx = Arc::clone(&self.context);
        let factory: Arc<dyn Fn(Arc<CardinalContext>) -> Result<T, CardinalError> + Send + Sync> =
            Arc::new(factory);
        self.context
            .register_with_factory::<T, _, _>(scope, move |_ctx| {
                let ctx_clone = Arc::clone(&ctx);
                let factory = Arc::clone(&factory);
                async move { (factory)(ctx_clone) }
            });
        self
    }

    pub fn register_singleton_instance<T>(self, instance: Arc<T>) -> Self
    where
        T: Provider + Send + Sync + 'static,
    {
        self.context.register_singleton_instance::<T>(instance);
        self
    }

    pub fn build(self) -> Cardinal {
        if self.auto_register_defaults {
            if !self.context.is_registered::<DestinationContainer>() {
                self.context
                    .register::<DestinationContainer>(ProviderScope::Singleton);
            }

            if !self.context.is_registered::<PluginContainer>() {
                self.context
                    .register::<PluginContainer>(ProviderScope::Singleton);
            }
        }

        Cardinal {
            context: self.context,
        }
    }
}
