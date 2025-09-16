pub mod internal;
pub mod proxy;

use crate::internal::CardinalInternalError;
use crate::proxy::CardinalProxyError;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Serialize, Deserialize, Error, Debug)]
pub enum CardinalError {
    #[error("Internal Error")]
    InternalError(#[from] CardinalInternalError),
    #[error("Proxy Error")]
    ProxyError(#[from] CardinalProxyError),
}
