use crate::plugin_executor::CardinalPluginExecutor;
use crate::request_context::RequestContext;
use async_trait::async_trait;
use cardinal_base::context::CardinalContext;
use cardinal_errors::CardinalError;
use pingora::http::ResponseHeader;
use pingora::proxy::Session;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MiddlewareResult {
    Continue(HashMap<String, String>),
    Responded,
}

#[async_trait]
pub trait RequestMiddleware: Send + Sync + 'static {
    async fn on_request(
        &self,
        session: &mut Session,
        req_ctx: &mut RequestContext,
        cardinal: Arc<CardinalContext>,
    ) -> Result<MiddlewareResult, CardinalError>;
}

#[async_trait]
pub trait ResponseMiddleware: Send + Sync + 'static {
    async fn on_response(
        &self,
        session: &mut Session,
        req_ctx: &mut RequestContext,
        response: &mut ResponseHeader,
        cardinal: Arc<CardinalContext>,
    );
}

pub type DynRequestMiddleware = dyn RequestMiddleware + Send + Sync + 'static;
pub type DynResponseMiddleware = dyn ResponseMiddleware + Send + Sync + 'static;

#[derive(Clone)]
pub struct PluginRunner {
    global_request: Arc<Vec<String>>,
    global_response: Arc<Vec<String>>,
    plugin_executor: Arc<dyn CardinalPluginExecutor>,
}

impl PluginRunner {
    pub fn new(
        context: Arc<CardinalContext>,
        plugin_executor: Arc<dyn CardinalPluginExecutor>,
    ) -> Self {
        let global_request = context.config.server.global_request_middleware.clone();
        let global_response = context.config.server.global_response_middleware.clone();

        Self {
            global_request: Arc::new(global_request),
            global_response: Arc::new(global_response),
            plugin_executor,
        }
    }

    fn global_request_filters(&self) -> &[String] {
        &self.global_request
    }

    fn global_response_filters(&self) -> &[String] {
        &self.global_response
    }

    pub async fn run_request_filters(
        &self,
        session: &mut Session,
        req_ctx: &mut RequestContext,
    ) -> Result<MiddlewareResult, CardinalError> {
        let mut resp_headers = HashMap::new();

        for filter in self.global_request_filters() {
            let run = self
                .plugin_executor
                .run_request_filter(filter, session, req_ctx)
                .await?;

            match run {
                MiddlewareResult::Continue(middleware_resp_headers) => {
                    resp_headers.extend(middleware_resp_headers)
                }
                MiddlewareResult::Responded => return Ok(MiddlewareResult::Responded),
            }
        }

        let backend = req_ctx.backend.clone(); // Cheap clone
        let inbound_middleware = backend.get_inbound_middleware();
        for middleware in inbound_middleware {
            let run = self
                .plugin_executor
                .run_request_filter(&middleware.name, session, req_ctx)
                .await?;

            match run {
                MiddlewareResult::Continue(middleware_resp_headers) => {
                    resp_headers.extend(middleware_resp_headers)
                }
                MiddlewareResult::Responded => return Ok(MiddlewareResult::Responded),
            }
        }

        Ok(MiddlewareResult::Continue(resp_headers))
    }

    pub async fn run_response_filters(
        &self,
        session: &mut Session,
        req_ctx: &mut RequestContext,
        response: &mut ResponseHeader,
    ) {
        for filter in self.global_response_filters() {
            let _ = self
                .plugin_executor
                .run_response_filter(filter, session, req_ctx, response)
                .await;
        }

        let backend = req_ctx.backend.clone(); // Cheap clone
        let outbound_middleware = backend.get_outbound_middleware();
        for middleware in outbound_middleware {
            let middleware_name = &middleware.name;
            let _ = self
                .plugin_executor
                .run_response_filter(middleware_name, session, req_ctx, response)
                .await;
        }
    }
}
