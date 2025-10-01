use crate::runner::PluginRunner;
use cardinal_base::context::CardinalContext;
use cardinal_base::destinations::container::DestinationWrapper;
use std::collections::HashMap;
use std::sync::Arc;

pub struct RequestContext {
    pub cardinal_context: Arc<CardinalContext>,
    pub backend: Arc<DestinationWrapper>,
    pub plugin_runner: Arc<PluginRunner>,
    pub response_headers: Option<HashMap<String, String>>,
    pub persistent_vars: HashMap<String, String>,
}

impl RequestContext {
    pub fn new(context: Arc<CardinalContext>, backend: Arc<DestinationWrapper>) -> Self {
        let runner = PluginRunner::new(context.clone());
        Self {
            cardinal_context: context,
            backend,
            plugin_runner: Arc::new(runner),
            response_headers: None,
            persistent_vars: HashMap::new(),
        }
    }
}
