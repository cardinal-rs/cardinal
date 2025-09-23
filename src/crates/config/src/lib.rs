use crate::config::get_config_builder;
use ::config::ConfigError;
use derive_builder::Builder;
use serde::{Deserialize, Serialize};
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
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            address: "0.0.0.0:1704".into(),
            force_path_parameter: true,
            log_upstream_response: true,
        }
    }
}

pub fn load_config(paths: &[String]) -> Result<CardinalConfig, ConfigError> {
    let builder = get_config_builder(paths)?;
    let config = builder.build()?.try_deserialize()?;

    Ok(config)
}
