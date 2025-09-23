mod restricted_route_filter;

use async_trait::async_trait;
use cardinal_base::destinations::container::DestinationWrapper;
use cardinal_errors::CardinalError;
use pingora::http::ResponseHeader;
use pingora::proxy::Session;
use restricted_route_filter::RestrictedRouteFilter;
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

pub type DynRequestFilter = dyn RequestFilter + Send + Sync + 'static;
pub type DynResponseFilter = dyn ResponseFilter + Send + Sync + 'static;

#[derive(Clone)]
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
            global_request_filters: Vec::new(),
            global_response_filters: Vec::new(),
        }
    }

    pub fn with_default_filters(mut self) -> Self {
        self.register_global_request(Arc::new(RestrictedRouteFilter));
        self
    }

    pub fn register_request(
        &mut self,
        name: impl Into<String>,
        filter: Arc<DynRequestFilter>,
    ) -> &mut Self {
        self.request_filters.insert(name.into(), filter);
        self
    }

    pub fn register_response(
        &mut self,
        name: impl Into<String>,
        filter: Arc<DynResponseFilter>,
    ) -> &mut Self {
        self.response_filters.insert(name.into(), filter);
        self
    }

    pub fn register_global_request(&mut self, filter: Arc<DynRequestFilter>) -> &mut Self {
        self.global_request_filters.push(filter);
        self
    }

    pub fn register_global_response(&mut self, filter: Arc<DynResponseFilter>) -> &mut Self {
        self.global_response_filters.push(filter);
        self
    }

    pub async fn run_request_filters(
        &self,
        session: &mut Session,
        backend: Arc<DestinationWrapper>,
    ) -> Result<FilterResult, CardinalError> {
        for filter in &self.global_request_filters {
            let res = filter.on_request(session, backend.clone()).await?;
            if let FilterResult::Responded = res {
                return Ok(FilterResult::Responded);
            }
        }

        let inbound_middleware = backend.get_inbound_middleware();
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
        for filter in &self.global_response_filters {
            filter.on_response(session, backend.clone(), response).await;
        }

        let outbound_middleware = backend.get_outbound_middleware();
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

impl Default for FilterRegistry {
    fn default() -> Self {
        Self::new().with_default_filters()
    }
}
