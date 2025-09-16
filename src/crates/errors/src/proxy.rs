use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Serialize, Deserialize, Error, Debug)]
pub enum CardinalProxyError {
    #[error("The URL is wrongly constructed")]
    BadUrl(String),
}
