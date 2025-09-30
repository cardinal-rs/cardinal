use crate::builtin::restricted_route_middleware::RestrictedRouteMiddleware;
use crate::runner::{DynRequestMiddleware, DynResponseMiddleware, MiddlewareResult};
use crate::utils::parse_query_string_multi;
use cardinal_base::context::CardinalContext;
use cardinal_base::destinations::container::DestinationWrapper;
use cardinal_base::provider::Provider;
use cardinal_config::Plugin;
use cardinal_errors::CardinalError;
use cardinal_wasm_plugins::plugin::WasmPlugin;
use cardinal_wasm_plugins::runner::{HostFunctionBuilder, HostFunctionMap, WasmRunner};
use cardinal_wasm_plugins::wasmer::{Function, FunctionEnv, Store};
use cardinal_wasm_plugins::{ExecutionContext, ResponseState};
use pingora::http::ResponseHeader;
use pingora::prelude::Session;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{error, warn};

pub enum PluginBuiltInType {
    Inbound(Arc<DynRequestMiddleware>),
    Outbound(Arc<DynResponseMiddleware>),
}

pub enum PluginHandler {
    Builtin(PluginBuiltInType),
    Wasm(Arc<WasmPlugin>),
}

pub struct PluginContainer {
    plugins: HashMap<String, Arc<PluginHandler>>,
    host_imports: HostFunctionMap,
}

impl PluginContainer {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::from_iter(Self::builtin_plugins()),
            host_imports: HashMap::new(),
        }
    }

    pub fn new_empty() -> Self {
        Self {
            plugins: HashMap::new(),
            host_imports: HashMap::new(),
        }
    }

    pub fn builtin_plugins() -> Vec<(String, Arc<PluginHandler>)> {
        vec![(
            "RestrictedRouteMiddleware".to_string(),
            Arc::new(PluginHandler::Builtin(PluginBuiltInType::Inbound(
                Arc::new(RestrictedRouteMiddleware),
            ))),
        )]
    }

    pub fn add_plugin(&mut self, name: String, plugin: PluginHandler) {
        self.plugins.insert(name, Arc::new(plugin));
    }

    pub fn remove_plugin(&mut self, name: &str) {
        self.plugins.remove(name);
    }

    pub fn add_host_function<F>(
        &mut self,
        namespace: impl Into<String>,
        name: impl Into<String>,
        builder: F,
    ) where
        F: Fn(&mut Store, &FunctionEnv<ExecutionContext>) -> Function + Send + Sync + 'static,
    {
        let ns = namespace.into();
        let host_entry = self.host_imports.entry(ns).or_default();
        let builder: HostFunctionBuilder = Arc::new(builder);
        host_entry.push((name.into(), builder));
    }

    pub fn extend_host_functions<I, S>(&mut self, namespace: S, functions: I)
    where
        I: IntoIterator<Item = (String, HostFunctionBuilder)>,
        S: Into<String>,
    {
        let ns = namespace.into();
        let host_entry = self.host_imports.entry(ns).or_default();
        host_entry.extend(functions);
    }

    fn host_imports(&self) -> Option<&HostFunctionMap> {
        if self.host_imports.is_empty() {
            None
        } else {
            Some(&self.host_imports)
        }
    }

    pub async fn run_request_filter(
        &self,
        name: &str,
        session: &mut Session,
        backend: Arc<DestinationWrapper>,
        ctx: Arc<CardinalContext>,
    ) -> Result<MiddlewareResult, CardinalError> {
        let plugin = self
            .plugins
            .get(name)
            .ok_or_else(|| CardinalError::Other(format!("Plugin {name} does not exist")))?;

        match plugin.as_ref() {
            PluginHandler::Builtin(builtin) => match builtin {
                PluginBuiltInType::Inbound(filter) => {
                    filter.on_request(session, backend, ctx).await
                }
                PluginBuiltInType::Outbound(_) => Err(CardinalError::Other(format!(
                    "The filter {name} is not a request filter"
                ))),
            },
            PluginHandler::Wasm(wasm) => {
                let get_req_headers = session
                    .req_header()
                    .headers
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
                    .collect();

                let query =
                    parse_query_string_multi(session.req_header().uri.query().unwrap_or(""));

                {
                    let runner = WasmRunner::new(wasm, self.host_imports());

                    let inbound_ctx = ExecutionContext::from_parts(
                        get_req_headers,
                        query,
                        None,
                        ResponseState::with_default_status(403),
                    );

                    let exec = runner.run(inbound_ctx)?;

                    if exec.should_continue {
                        Ok(MiddlewareResult::Continue)
                    } else {
                        let response_state = exec.execution_context.response();
                        let header_response = Self::build_response_header(response_state);

                        let _ = session.write_response_header(Box::new(header_response), true);

                        let _ = session.respond_error(response_state.status()).await;
                        Ok(MiddlewareResult::Responded)
                    }
                }
            }
        }
    }

    pub async fn run_response_filter(
        &self,
        name: &str,
        session: &mut Session,
        backend: Arc<DestinationWrapper>,
        response: &mut pingora::http::ResponseHeader,
        ctx: Arc<CardinalContext>,
    ) {
        let plugin = self
            .plugins
            .get(name)
            .ok_or_else(|| CardinalError::Other(format!("Plugin {name} does not exist")));

        if let Ok(plugin) = plugin {
            match plugin.as_ref() {
                PluginHandler::Builtin(builtin) => match builtin {
                    PluginBuiltInType::Inbound(_) => {
                        error!("The filter {name} is not a response filter");
                    }
                    PluginBuiltInType::Outbound(filter) => {
                        filter.on_response(session, backend, response, ctx).await
                    }
                },
                PluginHandler::Wasm(wasm) => {
                    let get_req_headers = session
                        .req_header()
                        .headers
                        .iter()
                        .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
                        .collect();
                    let query =
                        parse_query_string_multi(session.req_header().uri.query().unwrap_or(""));

                    {
                        let runner = WasmRunner::new(wasm, self.host_imports());

                        let outbound_ctx = ExecutionContext::from_parts(
                            get_req_headers,
                            query,
                            None,
                            ResponseState::default(),
                        );

                        let exec = runner.run(outbound_ctx);

                        match &exec {
                            Ok(ex) => {
                                let response_state = ex.execution_context.response();

                                for (key, val) in response_state.headers() {
                                    let _ =
                                        response.insert_header(key.to_string(), val.to_string());
                                }

                                if let Some(status) = response_state.status_override() {
                                    let _ = response.set_status(status);
                                }
                            }
                            Err(e) => {
                                error!("Failed to run plugin {}: {}", name, e);
                            }
                        }
                    }
                }
            }
        }
    }

    fn build_response_header(response: &ResponseState) -> ResponseHeader {
        let mut header = ResponseHeader::build(response.status(), None)
            .expect("failed to build response header");

        for (key, value) in response.headers() {
            let _ = header.insert_header(key.to_string(), value.to_string());
        }

        header
    }
}

impl Default for PluginContainer {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Provider for PluginContainer {
    async fn provide(ctx: &CardinalContext) -> Result<Self, CardinalError> {
        let preloaded_plugins = ctx.config.plugins.clone();
        let mut plugin_container = PluginContainer::new();

        for plugin in preloaded_plugins {
            let plugin_name = plugin.name();
            let plugin_exists = plugin_container.plugins.contains_key(plugin_name);

            if plugin_exists {
                warn!("Plugin {} already exists, skipping", plugin_name);
                continue;
            }

            match plugin {
                Plugin::Builtin(_) => continue,
                Plugin::Wasm(wasm_config) => {
                    let wasm_plugin = WasmPlugin::from_path(&wasm_config.path).map_err(|e| {
                        CardinalError::Other(format!(
                            "Failed to load plugin {}: {}",
                            wasm_config.name, e
                        ))
                    })?;
                    plugin_container.plugins.insert(
                        wasm_config.name.clone(),
                        Arc::new(PluginHandler::Wasm(Arc::new(wasm_plugin))),
                    );
                }
            }
        }

        Ok(plugin_container)
    }
}
