use cardinal_base::context::CardinalContext;
use cardinal_base::destinations::container::DestinationContainer;
use cardinal_base::provider::{Provider, ProviderScope};
use cardinal_config::{load_config, CardinalConfig};
use cardinal_errors::internal::CardinalInternalError;
use cardinal_errors::CardinalError;
use cardinal_proxy::filters::FilterRegistry;
use cardinal_proxy::CardinalProxy;
use pingora::prelude::Server;
use pingora::proxy::http_proxy_service;
use std::sync::Arc;

pub struct Cardinal {
    context: Arc<CardinalContext>,
    filters: Arc<FilterRegistry>,
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

    pub fn with_filters(mut self, filters: FilterRegistry) -> Self {
        let mut filters = filters;
        filters.ensure_default_filters();
        self.filters = Arc::new(filters);
        self
    }

    pub fn replace_filter_registry(&mut self, filters: FilterRegistry) {
        let mut filters = filters;
        filters.ensure_default_filters();
        self.filters = Arc::new(filters);
    }

    pub fn filters_mut(&mut self) -> &mut FilterRegistry {
        let registry = Arc::make_mut(&mut self.filters);
        registry.ensure_default_filters();
        registry
    }

    pub fn filters(&self) -> &Arc<FilterRegistry> {
        &self.filters
    }

    pub fn context(&self) -> &Arc<CardinalContext> {
        &self.context
    }

    pub fn run(&self) -> Result<(), CardinalError> {
        let mut server = Server::new(None).map_err(|e| {
            CardinalError::InternalError(CardinalInternalError::FailedToInitiateServer(
                e.to_string(),
            ))
        })?;
        server.bootstrap();

        let proxy = CardinalProxy::builder(self.context.clone())
            .with_filter_registry(self.filters.clone())
            .build();
        let mut proxy_service = http_proxy_service(&server.configuration, proxy);

        let server_addr = self.context.config.server.address.clone();

        proxy_service.add_tcp(&server_addr);

        tracing::info!(addr = %server_addr, "Listening on address");

        server.add_service(proxy_service);
        server.run_forever();
    }
}

pub struct CardinalBuilder {
    context: CardinalContext,
    filters: FilterRegistry,
}

impl CardinalBuilder {
    pub fn new(config: CardinalConfig) -> Self {
        let context = CardinalContext::new(config);
        context.register::<DestinationContainer>(ProviderScope::Singleton);

        Self {
            context,
            filters: FilterRegistry::default(),
        }
    }

    pub fn from_paths(config_paths: &[String]) -> Result<Self, CardinalError> {
        let config = load_config(config_paths)?;
        Ok(Self::new(config))
    }

    pub fn context(&self) -> &CardinalContext {
        &self.context
    }

    pub fn register_provider<T>(&self, scope: ProviderScope) -> &Self
    where
        T: Provider + Send + Sync + 'static,
    {
        self.context.register::<T>(scope);
        self
    }

    pub fn filters(&self) -> &FilterRegistry {
        &self.filters
    }

    pub fn filters_mut(&mut self) -> &mut FilterRegistry {
        self.filters.ensure_default_filters();
        &mut self.filters
    }

    pub fn set_filter_registry(&mut self, registry: FilterRegistry) -> &mut Self {
        let mut registry = registry;
        registry.ensure_default_filters();
        self.filters = registry;
        self
    }

    pub fn with_filter_registry(mut self, registry: FilterRegistry) -> Self {
        let mut registry = registry;
        registry.ensure_default_filters();
        self.filters = registry;
        self
    }

    pub fn build(self) -> Cardinal {
        let mut filters = self.filters;
        filters.ensure_default_filters();

        Cardinal {
            context: Arc::new(self.context),
            filters: Arc::new(filters),
        }
    }
}
