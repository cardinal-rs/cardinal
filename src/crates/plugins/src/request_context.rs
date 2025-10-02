use crate::runner::PluginRunner;
use cardinal_base::context::CardinalContext;
use cardinal_base::destinations::container::DestinationWrapper;
use cardinal_wasm_plugins::ExecutionContext;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::Arc;

pub struct RequestContext {
    pub cardinal_context: Arc<CardinalContext>,
    pub backend: Arc<DestinationWrapper>,
    pub plugin_runner: Arc<PluginRunner>,
    pub response_headers: Option<HashMap<String, String>>,
    pub plugin_exec_context: Arc<RwLock<ExecutionContext>>,
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
            plugin_exec_context: Arc::new(RwLock::new(execution_context)),
        }
    }

    pub fn persistent_vars(&self) -> Arc<RwLock<HashMap<String, String>>> {
        self.plugin_exec_context.read().persistent_vars().clone()
    }
}
