use cardinal_base::context::CardinalContext;
use cardinal_base::destinations::container::DestinationContainer;
use cardinal_base::provider::ProviderScope;
use cardinal_config::{load_config, CardinalConfig};
use cardinal_errors::internal::CardinalInternalError;
use cardinal_errors::CardinalError;
use cardinal_proxy::CardinalProxy;
use pingora::prelude::Server;
use pingora::proxy::http_proxy_service;
use std::sync::Arc;

pub struct Cardinal {
    context: Arc<CardinalContext>,
}

impl Cardinal {
    pub fn from_paths(config: &[String]) -> Result<Self, CardinalError> {
        let config = load_config(config)?;
        Ok(Self::new(config))
    }

    pub fn new(config: CardinalConfig) -> Self {
        let context = CardinalContext::new(config);
        context.register::<DestinationContainer>(ProviderScope::Singleton);

        Self {
            context: Arc::new(context),
        }
    }

    pub fn run(&self) -> Result<(), CardinalError> {
        let mut server = Server::new(None).map_err(|e| {
            CardinalError::InternalError(CardinalInternalError::FailedToInitiateServer(
                e.to_string(),
            ))
        })?;
        server.bootstrap();

        let proxy = CardinalProxy::new(self.context.clone());
        let mut proxy_service = http_proxy_service(&server.configuration, proxy);

        let server_addr = self.context.config.server.address.clone();

        proxy_service.add_tcp(&server_addr);

        tracing::info!(addr = %server_addr, "Listening on address");

        server.add_service(proxy_service);
        server.run_forever();
    }
}
