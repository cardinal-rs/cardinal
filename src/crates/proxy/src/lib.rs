mod utils;

use crate::utils::requests::{
    compose_upstream_url, parse_origin, rewrite_request_path, set_upstream_host_headers,
};
use cardinal_base::context::CardinalContext;
use cardinal_base::destinations::container::{DestinationContainer, DestinationWrapper};
use cardinal_plugins::runner::{MiddlewareResult, PluginRunner};
use pingora::http::ResponseHeader;
use pingora::prelude::*;
use pingora::protocols::Digest;
use pingora::upstreams::peer::Peer;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

pub mod pingora {
    pub use pingora::*;
}

pub trait CardinalContextProvider: Send + Sync {
    fn resolve(&self, session: &Session) -> Option<Arc<CardinalContext>>;
}

#[derive(Clone)]
pub struct StaticContextProvider {
    context: Arc<CardinalContext>,
}

impl StaticContextProvider {
    pub fn new(context: Arc<CardinalContext>) -> Self {
        Self { context }
    }
}

impl CardinalContextProvider for StaticContextProvider {
    fn resolve(&self, _session: &Session) -> Option<Arc<CardinalContext>> {
        Some(self.context.clone())
    }
}

pub struct CardinalProxy {
    provider: Arc<dyn CardinalContextProvider>,
}

impl CardinalProxy {
    pub fn new(context: Arc<CardinalContext>) -> Self {
        Self::builder(context).build()
    }

    pub fn with_provider(provider: Arc<dyn CardinalContextProvider>) -> Self {
        Self { provider }
    }

    pub fn builder(context: Arc<CardinalContext>) -> CardinalProxyBuilder {
        CardinalProxyBuilder::new(context)
    }
}

pub struct CardinalProxyBuilder {
    provider: Arc<dyn CardinalContextProvider>,
}

impl CardinalProxyBuilder {
    pub fn new(context: Arc<CardinalContext>) -> Self {
        Self {
            provider: Arc::new(StaticContextProvider::new(context)),
        }
    }

    pub fn from_context_provider(provider: Arc<dyn CardinalContextProvider>) -> Self {
        Self { provider }
    }

    pub fn with_context_provider(mut self, provider: Arc<dyn CardinalContextProvider>) -> Self {
        self.provider = provider;
        self
    }

    pub fn build(self) -> CardinalProxy {
        CardinalProxy::with_provider(self.provider)
    }
}

#[async_trait::async_trait]
impl ProxyHttp for CardinalProxy {
    type CTX = Option<RequestContext>;

    fn new_ctx(&self) -> Self::CTX {
        None
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        let path = session.req_header().uri.path().to_string();
        info!(%path, "Request received");

        let context = match self.provider.resolve(session) {
            Some(ctx) => ctx,
            None => {
                warn!(%path, "No context found for request host");
                let _ = session.respond_error(421).await;
                return Ok(true);
            }
        };

        let destination_container = context
            .get::<DestinationContainer>()
            .await
            .map_err(|_| Error::new_str("Destination Container is not present"))?;

        let force_path = context.config.server.force_path_parameter;
        let backend =
            match destination_container.get_backend_for_request(session.req_header(), force_path) {
                Some(b) => b.clone(),
                None => {
                    warn!(%path, "No matching backend, returning 404");
                    let _ = session.respond_error(404).await;
                    return Ok(true);
                }
            };

        let destination_name = backend.destination.name.clone();
        let _ = set_upstream_host_headers(session, &backend);
        info!(backend_id = %destination_name, "Routing to backend");

        rewrite_request_path(session.req_header_mut(), &destination_name, force_path);

        let mut request_state = RequestContext::new(context.clone());
        request_state.backend = Some(backend.clone());

        let run_filters = request_state
            .runner
            .run_request_filters(session, backend)
            .await;

        let res = match run_filters {
            Ok(filter_result) => filter_result,
            Err(err) => {
                error!(%err, "Error running request filters");
                let _ = session.respond_error(500).await;
                return Ok(true);
            }
        };

        *ctx = Some(request_state);

        match res {
            MiddlewareResult::Continue(resp_headers) => {
                ctx.as_mut().unwrap().response_headers = Some(resp_headers);

                Ok(false)
            }
            MiddlewareResult::Responded => Ok(true),
        }
    }

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        if let Some(state) = ctx.as_ref() {
            if let Some(backend) = state.backend.as_ref() {
                // Determine origin parts for TLS and SNI
                let (host, port, is_tls) = parse_origin(&backend.destination.url)
                    .map_err(|_| Error::new_str("Origin could not be parsed "))?;
                let hostport = format!("{host}:{port}");

                // Compose full upstream URL for logging with normalized scheme
                let path_and_query = _session
                    .req_header()
                    .uri
                    .path_and_query()
                    .map(|pq| pq.as_str())
                    .unwrap_or("/");
                let upstream_url = compose_upstream_url(is_tls, &host, port, path_and_query);

                info!(%upstream_url, backend_id = %backend.destination.name, is_tls, sni = %host, "Forwarding to upstream");
                debug!(upstream_origin = %hostport, "Connecting to upstream origin");

                let mut peer = HttpPeer::new(&hostport, is_tls, host);
                if let Some(opts) = peer.get_mut_peer_options() {
                    // Allow both HTTP/1.1 and HTTP/2 so plain HTTP backends keep working.
                    opts.set_http_version(2, 1);
                }
                let peer = Box::new(peer);
                Ok(peer)
            } else {
                Err(Error::new(ErrorType::InternalError))
            }
        } else {
            Err(Error::new(ErrorType::InternalError))
        }
    }

    async fn connected_to_upstream(
        &self,
        _session: &mut Session,
        reused: bool,
        peer: &HttpPeer,
        #[cfg(unix)] _fd: std::os::unix::io::RawFd,
        #[cfg(windows)] _sock: std::os::windows::io::RawSocket,
        _digest: Option<&Digest>,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        let backend_id = ctx
            .as_ref()
            .and_then(|state| state.backend.as_ref())
            .map(|b| b.destination.name.as_str())
            .unwrap_or("<unknown>");
        info!(backend_id, reused, peer = %peer, "Connected to upstream");
        Ok(())
    }

    async fn response_filter(
        &self,
        session: &mut Session,
        upstream_response: &mut ResponseHeader,
        ctx: &mut Self::CTX,
    ) -> Result<()> {
        if let Some(state) = ctx.as_mut() {
            if let Some(resp_headers) = state.response_headers.take() {
                for (key, val) in resp_headers {
                    let _ = upstream_response.insert_header(key, val);
                }
            }

            if let Some(backend) = state.backend.clone() {
                state
                    .runner
                    .run_response_filters(session, backend, upstream_response)
                    .await;
            }

            if !state.context.config.server.log_upstream_response {
                return Ok(());
            }

            let status = upstream_response.status.as_u16();
            let location = upstream_response
                .headers
                .get("location")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            let backend_id = state
                .backend
                .as_ref()
                .map(|b| b.destination.name.as_str())
                .unwrap_or("<unknown>");
            if let Some(loc) = location {
                info!(backend_id, status, location = %loc, "Upstream responded");
            } else {
                info!(backend_id, status, "Upstream responded");
            }
        }

        Ok(())
    }
}

pub struct RequestContext {
    pub context: Arc<CardinalContext>,
    pub backend: Option<Arc<DestinationWrapper>>,
    pub runner: PluginRunner,
    pub response_headers: Option<HashMap<String, String>>,
}

impl RequestContext {
    fn new(context: Arc<CardinalContext>) -> Self {
        let runner = PluginRunner::new(context.clone());
        Self {
            context,
            backend: None,
            runner,
            response_headers: None,
        }
    }
}
