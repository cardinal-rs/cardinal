mod restricted_route_filter;

use async_trait::async_trait;
use cardinal_base::destinations::container::DestinationWrapper;
use cardinal_errors::CardinalError;
use pingora::http::ResponseHeader;
use pingora::proxy::Session;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterResult {
    Continue,
    Responded,
}

#[async_trait]
pub trait RequestFilter: Send + Sync {
    async fn on_request(
        &self,
        session: &mut Session,
        backend: Arc<DestinationWrapper>,
    ) -> Result<FilterResult, CardinalError>;
}

#[async_trait]
pub trait ResponseFilter: Send + Sync {
    async fn on_response(
        &self,
        session: &mut Session,
        backend: Arc<DestinationWrapper>,
        response: &mut ResponseHeader,
    );
}

type DynRequestFilter = dyn RequestFilter + Send + Sync + 'static;
type DynResponseFilter = dyn ResponseFilter + Send + Sync + 'static;

pub struct FilterRegistry {
    request_filters: HashMap<String, Arc<DynRequestFilter>>,
    response_filters: HashMap<String, Arc<DynResponseFilter>>,
    global_request_filters: Vec<Arc<DynRequestFilter>>,
    global_response_filters: Vec<Arc<DynResponseFilter>>,
}

impl FilterRegistry {
    pub fn new() -> Self {
        Self {
            request_filters: HashMap::new(),
            response_filters: HashMap::new(),
        }
    }

    pub fn with_default_filters(&mut self) {}

    pub fn register_request(&mut self, name: impl Into<String>, filter: Arc<DynRequestFilter>) {
        self.request_filters.insert(name.into(), filter);
    }

    pub fn register_response(&mut self, name: impl Into<String>, filter: Arc<DynResponseFilter>) {
        self.response_filters.insert(name.into(), filter);
    }

    pub async fn run_request_filters(
        &self,
        session: &mut Session,
        backend: Arc<DestinationWrapper>,
    ) -> Result<FilterResult, CardinalError> {
        let inbound_middleware = backend.get_inbound_middleware();
        if inbound_middleware.is_empty() {
            return Ok(FilterResult::Continue);
        }

        for middleware in inbound_middleware {
            let middleware_name = &middleware.name;
            match self.request_filters.get(middleware_name) {
                Some(f) => {
                    let res = f.on_request(session, backend.clone()).await?;
                    match res {
                        FilterResult::Continue => {}
                        FilterResult::Responded => return Ok(FilterResult::Responded),
                    }
                }
                None => {
                    warn!(filter = %middleware_name, backend_id = %backend.destination.name, "Unknown middleware referenced; skipping");
                }
            }
        }

        Ok(FilterResult::Continue)
    }

    pub async fn run_response_filters(
        &self,
        session: &mut Session,
        backend: Arc<DestinationWrapper>,
        response: &mut ResponseHeader,
    ) {
        let outbound_middleware = backend.get_outbound_middleware();

        if outbound_middleware.is_empty() {
            return;
        }

        for middleware in outbound_middleware {
            let middleware_name = &middleware.name;
            match self.response_filters.get(middleware_name) {
                Some(f) => f.on_response(session, backend.clone(), response).await,
                None => {
                    warn!(filter = %middleware_name, backend_id = %backend.destination.name, "Unknown post-filter referenced; skipping")
                }
            }
        }
    }
}
