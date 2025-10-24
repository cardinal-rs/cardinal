use crate::config::get_config_builder;
use ::config::ConfigError;
use derive_builder::Builder;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::collections::BTreeMap;
use ts_rs::TS;

pub mod config;

#[derive(Debug, Clone, Serialize, Deserialize, Builder, TS)]
#[ts(export)]
pub struct HealthCheck {
    pub path: String,
    pub interval_ms: u64,
    pub timeout_ms: u64,
    pub expect_status: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS)]
#[ts(export)]
pub enum MiddlewareType {
    Inbound,
    Outbound,
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder, TS)]
#[ts(export)]
pub struct Middleware {
    pub r#type: MiddlewareType,
    pub name: String,
}

#[derive(Debug, Clone, TS)]
#[ts(export)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Builder, TS)]
#[ts(export)]
pub struct BuiltinPlugin {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder, TS)]
#[ts(export)]
pub struct WasmPluginConfig {
    pub name: String,
    pub path: String,
    pub memory_name: Option<String>,
    pub handle_name: Option<String>,
}

#[derive(Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
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
            Plugin::Builtin(builtin) => {
                #[derive(Serialize)]
                struct Wrapper<'a> {
                    builtin: &'a BuiltinPlugin,
                }
                Wrapper { builtin }.serialize(serializer)
            }
            Plugin::Wasm(wasm) => {
                #[derive(Serialize)]
                struct Wrapper<'a> {
                    wasm: &'a WasmPluginConfig,
                }
                Wrapper { wasm }.serialize(serializer)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export)]
pub enum DestinationMatchValue {
    String(String),
    Regex { regex: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Builder, TS)]
#[ts(export)]
pub struct DestinationMatch {
    pub host: Option<DestinationMatchValue>, // exact or wildcard “*.tenant.com”
    pub path_prefix: Option<DestinationMatchValue>, // e.g. “/billing/”
    pub path_exact: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Builder, TS, Default)]
#[ts(export)]
pub struct DestinationTimeouts {
    pub connect: Option<u64>,
    pub read: Option<u64>,
    pub write: Option<u64>,
    pub idle: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, TS, Default)]
#[ts(export)]
pub enum DestinationRetryBackoffType {
    Exponential,
    Linear,
    #[default]
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Builder, TS, Default)]
#[ts(export)]
pub struct DestinationRetry {
    pub max_attempts: u64,
    pub interval_ms: u64,
    pub backoff_type: DestinationRetryBackoffType,
    pub max_interval: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder, TS)]
#[ts(export)]
pub struct Destination {
    pub name: String,
    pub url: String,
    pub health_check: Option<HealthCheck>,
    #[serde(default)]
    pub default: bool,
    #[serde(default)]
    pub r#match: Option<Vec<DestinationMatch>>,
    #[serde(default)]
    pub routes: Vec<Route>,
    #[serde(default)]
    pub middleware: Vec<Middleware>,
    #[serde(default)]
    pub timeout: Option<DestinationTimeouts>,
    #[serde(default)]
    pub retry: Option<DestinationRetry>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder, TS)]
#[ts(export)]
pub struct ServerConfig {
    pub address: String,
    pub force_path_parameter: bool,
    pub log_upstream_response: bool,
    pub global_request_middleware: Vec<String>,
    pub global_response_middleware: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Builder, TS)]
#[ts(export)]
pub struct Route {
    pub path: String,
    pub method: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, Builder, TS)]
#[ts(export)]
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
    validate_config(&config)?;

    Ok(config)
}

pub fn validate_config(config: &CardinalConfig) -> Result<(), ConfigError> {
    if config
        .server
        .address
        .parse::<std::net::SocketAddr>()
        .is_err()
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use serde_json::{json, to_value};

    #[test]
    fn serialize_builtin_plugin() {
        let plugin = Plugin::Builtin(BuiltinPlugin {
            name: "Logger".to_string(),
        });

        let val = to_value(&plugin).unwrap();

        let expected = json!({
            "builtin": {
                "name": "Logger"
            }
        });

        assert_eq!(val, expected);
    }

    #[test]
    fn serialize_wasm_plugin() {
        let wasm_cfg = WasmPluginConfig {
            name: "RateLimit".to_string(),
            path: "plugins/ratelimit.wasm".to_string(),
            memory_name: None,
            handle_name: None,
        };
        let plugin = Plugin::Wasm(wasm_cfg);

        let val = to_value(&plugin).unwrap();

        let expected = json!({
            "wasm": {
                "name": "RateLimit",
                "path": "plugins/ratelimit.wasm",
                "memory_name": null,
                "handle_name": null
            }
        });

        assert_eq!(val, expected);
    }

    #[test]
    fn toml_builtin_plugin() {
        let plugin = Plugin::Builtin(BuiltinPlugin {
            name: "Logger".to_string(),
        });

        let toml_str = toml::to_string(&plugin).unwrap();

        let expected = r#"[builtin]
name = "Logger"
"#;

        assert_eq!(toml_str, expected);
    }

    #[test]
    fn toml_wasm_plugin() {
        let wasm_cfg = WasmPluginConfig {
            name: "RateLimit".to_string(),
            path: "plugins/ratelimit.wasm".to_string(),
            memory_name: None,
            handle_name: None,
        };
        let plugin = Plugin::Wasm(wasm_cfg);

        let toml_str = toml::to_string(&plugin).unwrap();

        // None fields are skipped
        let expected = r#"[wasm]
name = "RateLimit"
path = "plugins/ratelimit.wasm"
"#;

        assert_eq!(toml_str, expected);
    }

    #[test]
    fn destination_match_value_string_roundtrip_json() {
        let value = DestinationMatchValue::String("api.example.com".to_string());
        let serialized = to_value(&value).unwrap();

        assert_eq!(serialized, json!("api.example.com"));

        let from_string: DestinationMatchValue =
            serde_json::from_value(json!("api.example.com")).unwrap();
        assert_eq!(from_string, value);
    }

    #[test]
    fn destination_match_value_regex_roundtrip_json() {
        let value = DestinationMatchValue::Regex {
            regex: "^api\\.".to_string(),
        };
        let serialized = to_value(&value).unwrap();

        assert_eq!(serialized, json!({"regex": "^api\\."}));

        let decoded: DestinationMatchValue =
            serde_json::from_value(json!({"regex": "^api\\."})).unwrap();
        assert_eq!(decoded, value);
    }

    #[test]
    fn destination_match_value_string_roundtrip_toml() {
        let value = DestinationMatchValue::String("billing".to_string());
        #[derive(Serialize, Deserialize, Debug, PartialEq)]
        struct Wrapper {
            value: DestinationMatchValue,
        }

        let toml_encoded = toml::to_string(&Wrapper {
            value: value.clone(),
        })
        .unwrap();
        assert_eq!(toml_encoded, "value = \"billing\"\n");

        let decoded: Wrapper = toml::from_str(&toml_encoded).unwrap();
        assert_eq!(decoded.value, value);
    }

    #[test]
    fn destination_match_value_regex_roundtrip_toml() {
        let value = DestinationMatchValue::Regex {
            regex: "^/billing".to_string(),
        };
        #[derive(Serialize, Deserialize, Debug, PartialEq)]
        struct Wrapper {
            value: DestinationMatchValue,
        }

        let toml_encoded = toml::to_string(&Wrapper {
            value: value.clone(),
        })
        .unwrap();
        assert_eq!(toml_encoded, "[value]\nregex = \"^/billing\"\n");

        let decoded: Wrapper = toml::from_str(&toml_encoded).unwrap();
        assert_eq!(decoded.value, value);
    }

    #[test]
    fn destination_struct_match_variants() {
        let string_toml = r#"
name = "customer_service"
url = "https://svc.internal/api"

[[match]]
host = "support.example.com"
path_prefix = "/helpdesk"
"#;

        let customer: Destination = toml::from_str(string_toml).unwrap();
        let matcher = customer
            .r#match
            .as_ref()
            .and_then(|entries| entries.first())
            .expect("expected match section");
        assert_eq!(
            matcher.host,
            Some(DestinationMatchValue::String("support.example.com".into()))
        );
        assert_eq!(
            matcher.path_prefix,
            Some(DestinationMatchValue::String("/helpdesk".into()))
        );
        assert_eq!(matcher.path_exact, None);

        let regex_toml = r#"
name = "billing"
url = "https://billing.internal"

[[match]]
host = { regex = '^api\.(eu|us)\.example\.com$' }
path_prefix = { regex = '^/billing/(v\d+)/' }
"#;

        let billing: Destination = toml::from_str(regex_toml).unwrap();
        let matcher = billing
            .r#match
            .as_ref()
            .and_then(|entries| entries.first())
            .expect("expected match section");
        assert_eq!(
            matcher.host,
            Some(DestinationMatchValue::Regex {
                regex: r"^api\.(eu|us)\.example\.com$".into()
            })
        );
        assert_eq!(
            matcher.path_prefix,
            Some(DestinationMatchValue::Regex {
                regex: r"^/billing/(v\d+)/".into()
            })
        );
        assert_eq!(matcher.path_exact, None);
    }

    #[test]
    fn destination_match_toml_mixed_variants() {
        #[derive(Serialize, Deserialize, Debug, PartialEq)]
        struct ConfigHarness {
            destinations: BTreeMap<String, DestinationHarness>,
        }

        #[derive(Serialize, Deserialize, Debug, PartialEq)]
        struct DestinationHarness {
            name: String,
            url: String,
            #[serde(rename = "match")]
            matcher: Option<Vec<DestinationMatch>>,
        }

        impl DestinationHarness {
            fn first_match(&self) -> &DestinationMatch {
                self.matcher
                    .as_ref()
                    .and_then(|entries| entries.first())
                    .expect("matcher section present")
            }

            fn match_count(&self) -> usize {
                self.matcher.as_ref().map(|m| m.len()).unwrap_or(0)
            }
        }

        let toml_source = r#"
[destinations.customer_service]
name = "customer_service"
url = "https://svc.internal/api"

[[destinations.customer_service.match]]
host = "support.example.com"
path_prefix = "/helpdesk"

[[destinations.customer_service.match]]
host = "support.example.com"
path_prefix = { regex = '^/support' }

[destinations.billing]
name = "billing"
url = "https://billing.internal"

[[destinations.billing.match]]
host = { regex = '^api\.(eu|us)\.example\.com$' }
path_prefix = { regex = '^/billing/(v\d+)/' }
"#;

        let parsed: ConfigHarness = toml::from_str(toml_source).unwrap();

        let customer = parsed.destinations.get("customer_service").unwrap();
        assert_eq!(customer.match_count(), 2);
        let customer_match = customer.first_match();
        assert_eq!(
            customer_match.host,
            Some(DestinationMatchValue::String("support.example.com".into()))
        );
        assert_eq!(
            customer_match.path_prefix,
            Some(DestinationMatchValue::String("/helpdesk".into()))
        );

        let customer_matches = customer.matcher.as_ref().unwrap();
        let second = &customer_matches[1];
        assert_eq!(
            second.path_prefix,
            Some(DestinationMatchValue::Regex {
                regex: String::from("^/support"),
            })
        );

        let billing = parsed.destinations.get("billing").unwrap();
        assert_eq!(billing.match_count(), 1);
        let billing_match = billing.first_match();
        assert_eq!(
            billing_match.host,
            Some(DestinationMatchValue::Regex {
                regex: r"^api\.(eu|us)\.example\.com$".into()
            })
        );
        assert_eq!(
            billing_match.path_prefix,
            Some(DestinationMatchValue::Regex {
                regex: r"^/billing/(v\d+)/".into()
            })
        );

        let serialized = toml::to_string(&parsed).unwrap();
        let reparsed: ConfigHarness = toml::from_str(&serialized).unwrap();
        assert_eq!(reparsed, parsed);
    }

    #[test]
    fn destination_match_allows_empty_array() {
        #[derive(Serialize, Deserialize, Debug, PartialEq)]
        struct ConfigHarness {
            destinations: BTreeMap<String, DestinationHarness>,
        }

        #[derive(Serialize, Deserialize, Debug, PartialEq)]
        struct DestinationHarness {
            name: String,
            url: String,
            #[serde(rename = "match")]
            matcher: Option<Vec<DestinationMatch>>,
        }

        let toml_source = r#"
[destinations.empty]
name = "empty"
url = "https://empty.internal"
match = []
"#;

        let parsed: ConfigHarness = toml::from_str(toml_source).unwrap();
        let destination = parsed.destinations.get("empty").unwrap();
        assert!(destination
            .matcher
            .as_ref()
            .map(|entries| entries.is_empty())
            .unwrap_or(false));

        let serialized = toml::to_string(&parsed).unwrap();
        let reparsed: ConfigHarness = toml::from_str(&serialized).unwrap();
        assert_eq!(reparsed, parsed);
    }
}
