use crate::headers::CARDINAL_PARAMS_HEADER_BASE;
use crate::request_context::RequestContext;
use crate::runner::{MiddlewareResult, RequestMiddleware};
use cardinal_base::context::CardinalContext;
use cardinal_errors::CardinalError;
use pingora::proxy::Session;
use std::collections::HashMap;
use std::sync::Arc;

pub struct RestrictedRouteMiddleware;

#[async_trait::async_trait]
impl RequestMiddleware for RestrictedRouteMiddleware {
    async fn on_request(
        &self,
        session: &mut Session,
        req_ctx: &mut RequestContext,
        _cardinal: Arc<CardinalContext>,
    ) -> Result<MiddlewareResult, CardinalError> {
        if req_ctx.backend.has_routes {
            let req_header = session.req_header();
            let method = req_header.method.as_str().to_lowercase();
            let validate = req_ctx.backend.router.valid(&method, req_header.uri.path());
            if let Some((valid, params)) = validate {
                if valid {
                    let req_header = session.req_header_mut();
                    for (k, v) in params {
                        req_header
                            .insert_header(format!("{CARDINAL_PARAMS_HEADER_BASE}{k}"), v)
                            .unwrap();
                    }
                }

                Ok(MiddlewareResult::Continue(HashMap::new()))
            } else {
                let _ = session.respond_error(402).await;
                Ok(MiddlewareResult::Responded)
            }
        } else {
            Ok(MiddlewareResult::Continue(HashMap::new()))
        }
    }
}
