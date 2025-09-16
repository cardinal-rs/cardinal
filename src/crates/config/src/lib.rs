use crate::config::get_config_builder;
use ::config::ConfigError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub mod config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    pub path: String,
    pub interval_ms: u64,
    pub timeout_ms: u64,
    pub expect_status: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Destination {
    pub name: String,
    pub url: String,
    pub health_check: Option<HealthCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Route {
    pub name: String,
    pub destination: String,
    pub host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CardinalConfig {
    pub server: ServerConfig,
    pub destinations: BTreeMap<String, Destination>,
    pub routes: BTreeMap<String, Route>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        ServerConfig {
            address: "0.0.0.0:1704".into(),
        }
    }
}

pub fn load_config(paths: &[String]) -> Result<CardinalConfig, ConfigError> {
    let builder = get_config_builder(paths)?;
    let config = builder.build()?.try_deserialize()?;

    Ok(config)
}
