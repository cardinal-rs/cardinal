use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Serialize, Deserialize, Error, Debug)]
pub enum CardinalInternalError {
    #[error("A dependency was found but did not match T")]
    DependencyTypeMismatch,
    #[error("Provider was called but could errored")]
    ProviderNotBuilt,
    #[error("Provider is dependent while being constructed")]
    DependencyCycleDetected,
    #[error("No provider registered for requested type")]
    ProviderNotRegistered,
    #[error("Unknown error {0}")]
    FailedToInitiateServer(String),
    #[error("Invalid Route Configuration {0}")]
    InvalidRouteConfiguration(String),
}
