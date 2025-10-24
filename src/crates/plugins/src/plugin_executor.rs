use crate::container::PluginContainer;
use crate::request_context::RequestContext;
use crate::runner::MiddlewareResult;
use cardinal_errors::CardinalError;
use pingora::prelude::Session;
use pingora::BError;
use std::sync::Arc;

#[async_trait::async_trait]
pub trait CardinalPluginExecutor: Send + Sync {
    async fn get_plugin_container(
        &self,
        _session: &mut Session,
        req_ctx: &mut RequestContext,
    ) -> pingora::Result<Arc<PluginContainer>, CardinalError>
    where
        Self: Send + Sync,
    {
        let filter_container = req_ctx.cardinal_context.get::<PluginContainer>().await?;
        Ok(filter_container)
    }

    async fn can_run_plugin(
        &self,
        _binding_id: &str,
        _session: &mut Session,
        _req_ctx: &mut RequestContext,
    ) -> Result<bool, BError>
    where
        Self: Send + Sync,
    {
        Ok(true)
    }

    async fn run_request_filter(
        &self,
        name: &str,
        session: &mut Session,
        req_ctx: &mut RequestContext,
    ) -> Result<MiddlewareResult, CardinalError> {
        let plugin_container = self.get_plugin_container(session, req_ctx).await?;
        plugin_container
            .run_request_filter(name, session, req_ctx)
            .await
    }

    async fn run_response_filter(
        &self,
        name: &str,
        session: &mut Session,
        req_ctx: &mut RequestContext,
        response: &mut pingora::http::ResponseHeader,
    ) -> Result<(), CardinalError> {
        let plugin_container = self.get_plugin_container(session, req_ctx).await?;

        plugin_container
            .run_response_filter(name, session, req_ctx, response)
            .await;

        Ok(())
    }
}
