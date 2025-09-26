use crate::context::CardinalContext;
use async_trait::async_trait;
use cardinal_errors::CardinalError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderScope {
    Singleton,
    Transient,
}

pub type DefaultProviderError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[async_trait]
pub trait Provider: Send + Sync + Sized + 'static {
    async fn provide(ctx: &CardinalContext) -> Result<Self, CardinalError>;
}
