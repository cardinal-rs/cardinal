use crate::retry::RetryState;
use cardinal_plugins::request_context::{RequestContext, RequestContextBase};

#[derive(Default)]
pub struct ReqCtx {
    pub ctx_base: RequestContextBase,
    pub retry_state: Option<RetryState>,
}

impl ReqCtx {
    pub fn req_unsafe(&self) -> &RequestContext {
        self.ctx_base.req_unsafe()
    }

    pub fn req_unsafe_mut(&mut self) -> &mut RequestContext {
        self.ctx_base.req_unsafe_mut()
    }

    pub fn set_resolved_request(&mut self, resolved_request: RequestContext) {
        self.ctx_base.set_resolved_request(resolved_request);
    }

    pub fn set(&mut self, key: &str, value: &str) {
        self.ctx_base.set(key, value);
    }
}
