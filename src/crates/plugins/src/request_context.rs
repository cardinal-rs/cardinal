use crate::runner::PluginRunner;
use crate::REQ_UTC_TIME;
use cardinal_base::context::CardinalContext;
use cardinal_base::destinations::container::DestinationWrapper;
use cardinal_wasm_plugins::{ExecutionContext, SharedExecutionContext};
use chrono::Utc;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

pub struct RequestContext {
    pub cardinal_context: Arc<CardinalContext>,
    pub backend: Arc<DestinationWrapper>,
    pub plugin_runner: Arc<PluginRunner>,
    pub response_headers: Option<HashMap<String, String>>,
    pub shared_ctx: SharedExecutionContext,
}

impl RequestContext {
    pub fn new(
        context: Arc<CardinalContext>,
        backend: Arc<DestinationWrapper>,
        execution_context: ExecutionContext,
    ) -> Self {
        let runner = PluginRunner::new(context.clone());
        Self {
            cardinal_context: context,
            backend,
            plugin_runner: Arc::new(runner),
            response_headers: None,
            shared_ctx: Arc::new(RwLock::new(execution_context)),
        }
    }

    pub fn persistent_vars(&self) -> Arc<RwLock<HashMap<String, String>>> {
        self.shared_ctx.read().persistent_vars().clone()
    }

    pub fn shared_context(&self) -> SharedExecutionContext {
        self.shared_ctx.clone()
    }
}

pub struct RequestContextBase {
    pub resolved_request: Option<RequestContext>,
    pub metadata: HashMap<String, String>,
    pub req_instant: Instant,
}

impl Default for RequestContextBase {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestContextBase {
    pub fn new() -> Self {
        Self {
            resolved_request: None,
            metadata: Self::init_metadata(),
            req_instant: Instant::now(),
        }
    }

    pub fn set(&mut self, key: &str, value: &str) {
        self.metadata.insert(key.to_string(), value.to_string());
    }

    pub fn set_resolved_request(&mut self, resolved_request: RequestContext) {
        self.resolved_request = Some(resolved_request);
    }

    pub fn req_unsafe(&self) -> &RequestContext {
        self.resolved_request.as_ref().unwrap()
    }

    pub fn req_unsafe_mut(&mut self) -> &mut RequestContext {
        self.resolved_request.as_mut().unwrap()
    }

    fn init_metadata() -> HashMap<String, String> {
        HashMap::from([(REQ_UTC_TIME.to_string(), Utc::now().to_rfc3339())])
    }
}
