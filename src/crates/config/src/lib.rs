use crate::config::get_config_builder;
use ::config::ConfigError;
use derive_builder::Builder;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;

pub mod config;

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct HealthCheck {
    pub path: String,
    pub interval_ms: u64,
    pub timeout_ms: u64,
    pub expect_status: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum MiddlewareType {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct Middleware {
    pub r#type: MiddlewareType,
    pub name: String,
}

#[derive(Debug, Clone)]
pub enum Plugin {
    Builtin(BuiltinPlugin),
    Wasm(WasmPluginConfig),
}

impl Plugin {
    pub fn name(&self) -> &str {
        match self {
            Plugin::Builtin(builtin) => &builtin.name,
            Plugin::Wasm(wasm) => &wasm.name,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct BuiltinPlugin {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct WasmPluginConfig {
    pub name: String,
    pub path: String,
    pub memory_name: Option<String>,
    pub handle_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum PluginSerde {
    Name(String),
    Builtin { builtin: BuiltinPlugin },
    Wasm { wasm: WasmPluginConfig },
}

impl<'de> Deserialize<'de> for Plugin {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        match PluginSerde::deserialize(deserializer)? {
            PluginSerde::Name(name) => Ok(Plugin::Builtin(BuiltinPlugin { name })),
            PluginSerde::Builtin { builtin } => Ok(Plugin::Builtin(builtin)),
            PluginSerde::Wasm { wasm } => Ok(Plugin::Wasm(wasm)),
        }
    }
}

impl Serialize for Plugin {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Plugin::Builtin(builtin) => BuiltinPlugin::serialize(builtin, serializer),
            Plugin::Wasm(wasm) => WasmPluginConfig::serialize(wasm, serializer),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct Destination {
    pub name: String,
    pub url: String,
    pub health_check: Option<HealthCheck>,
    #[serde(default)]
    pub routes: Vec<Route>,
    #[serde(default)]
    pub middleware: Vec<Middleware>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct ServerConfig {
    pub address: String,
    pub force_path_parameter: bool,
    pub log_upstream_response: bool,
    pub global_request_middleware: Vec<String>,
    pub global_response_middleware: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder)]
pub struct Route {
    pub path: String,
    pub method: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Builder)]
pub struct CardinalConfig {
    pub server: ServerConfig,
    pub destinations: BTreeMap<String, Destination>,
    #[serde(default)]
    pub plugins: Vec<Plugin>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            address: "0.0.0.0:1704".into(),
            force_path_parameter: true,
            log_upstream_response: true,
            global_response_middleware: vec![],
            global_request_middleware: vec![],
        }
    }
}

pub fn load_config(paths: &[String]) -> Result<CardinalConfig, ConfigError> {
    let builder = get_config_builder(paths)?;
    let config: CardinalConfig = builder.build()?.try_deserialize()?;
    let _ = validate_config(&config)?;

    Ok(config)
}

pub fn validate_config(config: &CardinalConfig) -> Result<(), ConfigError> {
    if !config
        .server
        .address
        .parse::<std::net::SocketAddr>()
        .is_ok()
    {
        return Err(ConfigError::Message(format!(
            "Invalid server address: {}",
            config.server.address
        )));
    }

    let all_plugin_names = config
        .plugins
        .iter()
        .map(|p| p.name())
        .collect::<Vec<&str>>();

    for middleware in config.destinations.values().flat_map(|d| &d.middleware) {
        if !all_plugin_names.contains(&middleware.name.as_str()) {
            return Err(ConfigError::Message(format!(
                "Middleware {} not found. {0} must be included in the list of plugins.",
                middleware.name
            )));
        }
    }

    for destination in config.destinations.values() {
        for route in &destination.routes {
            if !route.path.starts_with('/') {
                return Err(ConfigError::Message(format!(
                    "Route path {} must start with a '/'.",
                    route.path
                )));
            }
        }
    }

    for destination in config.destinations.values() {
        for route in &destination.routes {
            if !route.method.eq("GET")
                && !route.method.eq("POST")
                && !route.method.eq("PUT")
                && !route.method.eq("DELETE")
                && !route.method.eq("PATCH")
                && !route.method.eq("HEAD")
                && !route.method.eq("OPTIONS")
            {
                return Err(ConfigError::Message(format!(
                    "Route method {} is not supported.",
                    route.method
                )));
            }
        }
    }

    Ok(())
}
