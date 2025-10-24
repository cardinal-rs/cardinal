use crate::pingora::{BError, Error};
use crate::req::ReqCtx;
use crate::HealthCheckStatus;
use cardinal_base::context::CardinalContext;
use pingora::proxy::Session;
use std::sync::Arc;

#[async_trait::async_trait]
pub trait CardinalContextProvider: Send + Sync {
    fn ctx(&self) -> ReqCtx {
        ReqCtx::default()
    }

    fn resolve(&self, session: &Session, ctx: &mut ReqCtx) -> Option<Arc<CardinalContext>>;
    fn health_check(&self, _session: &Session) -> HealthCheckStatus {
        HealthCheckStatus::None
    }

    fn logging(&self, _session: &mut Session, _e: Option<&Error>, _ctx: &mut ReqCtx) {}

    async fn early_request_filter(
        &self,
        _session: &mut Session,
        _ctx: &mut ReqCtx,
    ) -> Result<(), BError>
    where
        Self: Send + Sync,
    {
        Ok(())
    }
}
