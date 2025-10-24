pub mod internal;
pub mod proxy;

use crate::internal::CardinalInternalError;
use crate::proxy::CardinalProxyError;
use config::ConfigError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CardinalError {
    #[error("Internal Error {0}")]
    InternalError(#[from] CardinalInternalError),
    #[error("Proxy Error")]
    ProxyError(#[from] CardinalProxyError),
    #[error("Config Error {0}")]
    InvalidConfig(#[from] ConfigError),
    #[error("IO Error")]
    IoError(#[from] std::io::Error),
    #[error("Other Error {0}")]
    Other(String),
}
