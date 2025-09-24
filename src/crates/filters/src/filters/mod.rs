use crate::container::PluginContainer;
use async_trait::async_trait;
use cardinal_base::context::CardinalContext;
use cardinal_base::destinations::container::DestinationWrapper;
use cardinal_errors::CardinalError;
use pingora::http::ResponseHeader;
use pingora::proxy::Session;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MiddlewareResult {
    Continue,
    Responded,
}

#[async_trait]
pub trait RequestMiddleware: Send + Sync + 'static {
    async fn on_request(
        &self,
        session: &mut Session,
        backend: Arc<DestinationWrapper>,
        cardinal: Arc<CardinalContext>,
    ) -> Result<MiddlewareResult, CardinalError>;
}

#[async_trait]
pub trait ResponseMiddleware: Send + Sync + 'static {
    async fn on_response(
        &self,
        session: &mut Session,
        backend: Arc<DestinationWrapper>,
        response: &mut ResponseHeader,
        cardinal: Arc<CardinalContext>,
    );
}

pub type DynRequestMiddleware = dyn RequestMiddleware + Send + Sync + 'static;
pub type DynResponseMiddleware = dyn ResponseMiddleware + Send + Sync + 'static;

#[derive(Clone)]
pub struct FilterRegistry {
    context: Arc<CardinalContext>,
}

impl FilterRegistry {
    pub fn new(context: Arc<CardinalContext>) -> Self {
        Self { context }
    }

    fn global_request_filters(&self) -> Vec<String> {
        self.context.config.server.global_request_middleware.clone()
    }

    fn global_response_filters(&self) -> Vec<String> {
        self.context
            .config
            .server
            .global_response_middleware
            .clone()
    }

    pub async fn run_request_filters(
        &self,
        session: &mut Session,
        backend: Arc<DestinationWrapper>,
    ) -> Result<MiddlewareResult, CardinalError> {
        let filter_container = self.context.get::<PluginContainer>().await?;

        for filter in self.global_request_filters() {
            let run = filter_container
                .run_request_filter(&filter, session, backend.clone(), self.context.clone())
                .await?;
            if let MiddlewareResult::Responded = run {
                return Ok(MiddlewareResult::Responded);
            }
        }

        let inbound_middleware = backend.get_inbound_middleware();
        for middleware in inbound_middleware {
            let middleware_name = &middleware.name;
            let run = filter_container
                .run_request_filter(
                    middleware_name,
                    session,
                    backend.clone(),
                    self.context.clone(),
                )
                .await?;
            if let MiddlewareResult::Responded = run {
                return Ok(MiddlewareResult::Responded);
            }
        }

        Ok(MiddlewareResult::Continue)
    }

    pub async fn run_response_filters(
        &self,
        session: &mut Session,
        backend: Arc<DestinationWrapper>,
        response: &mut ResponseHeader,
    ) {
        let filter_container = self.context.get::<PluginContainer>().await.unwrap();

        for filter in self.global_response_filters() {
            filter_container
                .run_response_filter(
                    &filter,
                    session,
                    backend.clone(),
                    response,
                    self.context.clone(),
                )
                .await;
        }

        let outbound_middleware = backend.get_outbound_middleware();
        for middleware in outbound_middleware {
            let middleware_name = &middleware.name;
            filter_container
                .run_response_filter(
                    &middleware_name,
                    session,
                    backend.clone(),
                    response,
                    self.context.clone(),
                )
                .await;
        }
    }
}
