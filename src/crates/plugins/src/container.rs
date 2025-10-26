use crate::builtin::restricted_route_middleware::RestrictedRouteMiddleware;
use crate::request_context::RequestContext;
use crate::runner::{DynRequestMiddleware, DynResponseMiddleware, MiddlewareResult};
use cardinal_base::context::CardinalContext;
use cardinal_base::provider::Provider;
use cardinal_config::Plugin;
use cardinal_errors::CardinalError;
use cardinal_wasm_plugins::host::{HostFunctionBuilder, HostImportHandle};
use cardinal_wasm_plugins::plugin::WasmPlugin;
use cardinal_wasm_plugins::runner::{host_import_from_builder, ExecutionPhase, WasmRunner};
use cardinal_wasm_plugins::wasmer::{Function, FunctionEnv, Store};
use cardinal_wasm_plugins::{ResponseState, SharedExecutionContext};
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
    host_imports: Vec<HostImportHandle>,
}

impl PluginContainer {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::from_iter(Self::builtin_plugins()),
            host_imports: Vec::new(),
        }
    }

    pub fn new_empty() -> Self {
        Self {
            plugins: HashMap::new(),
            host_imports: Vec::new(),
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
        F: Fn(&mut Store, &FunctionEnv<SharedExecutionContext>) -> Function + Send + Sync + 'static,
    {
        let builder: HostFunctionBuilder = Arc::new(builder);
        let import = host_import_from_builder(namespace, name, builder);
        self.host_imports.push(import);
    }

    pub fn extend_host_functions<I>(&mut self, functions: I)
    where
        I: IntoIterator<Item = HostImportHandle>,
    {
        self.host_imports.extend(functions);
    }

    fn host_imports(&self) -> Option<&[HostImportHandle]> {
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
        req_ctx: &mut RequestContext,
    ) -> Result<MiddlewareResult, CardinalError> {
        let plugin = self
            .plugins
            .get(name)
            .ok_or_else(|| CardinalError::Other(format!("Plugin {name} does not exist")))?;

        match plugin.as_ref() {
            PluginHandler::Builtin(builtin) => match builtin {
                PluginBuiltInType::Inbound(filter) => {
                    filter
                        .on_request(session, req_ctx, req_ctx.cardinal_context.clone())
                        .await
                }
                PluginBuiltInType::Outbound(_) => Err(CardinalError::Other(format!(
                    "The filter {name} is not a request filter"
                ))),
            },
            PluginHandler::Wasm(wasm) => {
                let runner = WasmRunner::new(wasm, ExecutionPhase::Inbound, self.host_imports());

                let exec = runner.run(req_ctx.shared_context())?;
                let should_continue = exec.should_continue;

                let (header_updates, response_snapshot) = {
                    let guard = exec.execution_context.read();
                    let request_headers: Vec<(String, String)> = guard
                        .request()
                        .headers()
                        .iter()
                        .filter_map(|(key, value)| {
                            value
                                .to_str()
                                .ok()
                                .map(|v| (key.as_str().to_string(), v.to_string()))
                        })
                        .collect();

                    let response_state = guard.response().clone();
                    (request_headers, response_state)
                };

                if !header_updates.is_empty() {
                    for (key, val) in header_updates {
                        let _ = session.req_header_mut().insert_header(key, val);
                    }
                }

                if !should_continue || response_snapshot.status_override().is_some() {
                    let state = Self::build_response_header(&response_snapshot);
                    Ok(Self::respond_from_response_state(state, response_snapshot.status(), session).await)
                } else {
                    let headers: HashMap<String, String> = response_snapshot
                        .headers()
                        .iter()
                        .filter_map(|(key, value)| {
                            value
                                .to_str()
                                .ok()
                                .map(|v| (key.as_str().to_string(), v.to_string()))
                        })
                        .collect();
                    Ok(MiddlewareResult::Continue(headers))
                }
            }
        }
    }

    pub async fn run_response_filter(
        &self,
        name: &str,
        session: &mut Session,
        req_ctx: &mut RequestContext,
        response: &mut pingora::http::ResponseHeader,
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
                        filter
                            .on_response(
                                session,
                                req_ctx,
                                response,
                                req_ctx.cardinal_context.clone(),
                            )
                            .await
                    }
                },
                PluginHandler::Wasm(wasm) => {
                    let runner =
                        WasmRunner::new(wasm, ExecutionPhase::Outbound, self.host_imports());

                    match runner.run(req_ctx.shared_context()) {
                        Ok(exec) => {
                            let snapshot = {
                                let guard = exec.execution_context.read();
                                guard.response().clone()
                            };

                            for (key, val) in snapshot.headers().iter() {
                                let _ = response.insert_header(key.clone(), val.clone());
                            }

                            if let Some(status) = snapshot.status_override() {
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

    pub fn build_response_header(response: &ResponseState) -> ResponseHeader {
        let mut header = ResponseHeader::build(response.status(), None)
            .expect("failed to build response header");

        for (key, value) in response.headers().iter() {
            let _ = header.insert_header(key.clone(), value.clone());
        }

        header
    }

    pub async fn respond_from_response_state(response_header: ResponseHeader, status: u16, session: &mut Session) -> MiddlewareResult {
        let _ = session
            .write_response_header(Box::new(response_header), false)
            .await;
        let _ = session.respond_error(status).await;

        MiddlewareResult::Responded
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
