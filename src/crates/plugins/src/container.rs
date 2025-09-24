use crate::builtin::restricted_route_middleware::RestrictedRouteMiddleware;
use crate::runner::{DynRequestMiddleware, DynResponseMiddleware, MiddlewareResult};
use crate::utils::parse_query_string_multi;
use cardinal_base::context::CardinalContext;
use cardinal_base::destinations::container::DestinationWrapper;
use cardinal_base::provider::Provider;
use cardinal_config::Plugin;
use cardinal_errors::CardinalError;
use cardinal_wasm_plugins::plugin::WasmPlugin;
use cardinal_wasm_plugins::runner::WasmRunner;
use cardinal_wasm_plugins::ExecutionContext;
use pingora::prelude::Session;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::error;

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
}

impl PluginContainer {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::from_iter(Self::builtin_plugins()),
        }
    }

    pub fn new_empty() -> Self {
        Self {
            plugins: HashMap::new(),
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
            .ok_or_else(|| CardinalError::Other(format!("Plugin {} does not exist", name)))?;

        match plugin.as_ref() {
            PluginHandler::Builtin(builtin) => match builtin {
                PluginBuiltInType::Inbound(filter) => {
                    filter.on_request(session, backend, ctx).await
                }
                PluginBuiltInType::Outbound(_) => Err(CardinalError::Other(format!(
                    "The filter {} is not a request filter",
                    name
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
                    let runner = WasmRunner::new(wasm);
                    let exec = runner.run(ExecutionContext {
                        memory: None,
                        req_headers: get_req_headers,
                        query,
                        resp_headers: Default::default(),
                        status: 0,
                        body: None,
                    })?;

                    if exec.should_continue {
                        return Ok(MiddlewareResult::Continue);
                    }
                }

                Ok(MiddlewareResult::Continue)
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
            .ok_or_else(|| CardinalError::Other(format!("Plugin {} does not exist", name)));

        if let Ok(plugin) = plugin {
            match plugin.as_ref() {
                PluginHandler::Builtin(builtin) => match builtin {
                    PluginBuiltInType::Inbound(_) => {
                        error!("The filter {} is not a response filter", name);
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
                        let runner = WasmRunner::new(wasm);
                        let exec = runner.run(ExecutionContext {
                            memory: None,
                            req_headers: get_req_headers,
                            query,
                            resp_headers: Default::default(),
                            status: 0,
                            body: None,
                        });

                        if let Ok(exec) = exec {
                            if exec.should_continue {
                                return;
                            }
                        } else if let Err(e) = exec {
                            error!("Failed to run plugin {}: {}", name, e);
                        }
                    }
                }
            }
        }
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

            if !plugin_exists {
                return Err(CardinalError::Other(format!(
                    "Failed to initiate plugin container. Plugin {} does not exist",
                    plugin_name
                )));
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
