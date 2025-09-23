pub mod filters;
mod headers;
mod utils;

use crate::filters::{FilterRegistry, FilterResult};
use crate::utils::requests::{
    compose_upstream_url, parse_origin, rewrite_request_path, set_upstream_host_headers,
};
use cardinal_base::context::CardinalContext;
use cardinal_base::destinations::container::{DestinationContainer, DestinationWrapper};
use pingora::http::ResponseHeader;
use pingora::prelude::*;
use pingora::protocols::Digest;
use pingora::upstreams::peer::Peer;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

pub struct CardinalProxy {
    context: Arc<CardinalContext>,
    filters: Arc<FilterRegistry>,
}

impl CardinalProxy {
    pub fn new(context: Arc<CardinalContext>) -> Self {
        Self::builder(context).build()
    }

    pub fn builder(context: Arc<CardinalContext>) -> CardinalProxyBuilder {
        CardinalProxyBuilder::new(context)
    }

    pub fn filters(&self) -> &FilterRegistry {
        self.filters.as_ref()
    }

    pub fn filters_mut(&mut self) -> &mut FilterRegistry {
        let registry = Arc::make_mut(&mut self.filters);
        registry.set_context(self.context.clone());
        registry.ensure_default_filters();
        registry
    }
}

pub struct CardinalProxyBuilder {
    context: Arc<CardinalContext>,
    filters: FilterRegistry,
}

impl CardinalProxyBuilder {
    pub fn new(context: Arc<CardinalContext>) -> Self {
        let filters = FilterRegistry::new(context.clone()).with_default_filters();

        Self { context, filters }
    }

    pub fn with_filter_registry(mut self, filters: FilterRegistry) -> Self {
        self.filters = filters.with_context(self.context.clone());
        self.filters.ensure_default_filters();
        self
    }

    pub fn with_shared_filter_registry(mut self, filters: Arc<FilterRegistry>) -> Self {
        let registry = (*filters).clone().with_context(self.context.clone());
        self.filters = registry;
        self.filters.ensure_default_filters();
        self
    }

    pub fn with_owned_filter_registry(self, filters: FilterRegistry) -> Self {
        self.with_filter_registry(filters)
    }

    pub fn filters(&self) -> &FilterRegistry {
        &self.filters
    }

    pub fn filters_mut(&mut self) -> &mut FilterRegistry {
        self.filters.set_context(self.context.clone());
        self.filters.ensure_default_filters();
        &mut self.filters
    }

    pub fn build(self) -> CardinalProxy {
        let mut filters = self.filters;
        filters.set_context(self.context.clone());
        filters.ensure_default_filters();

        CardinalProxy {
            context: self.context,
            filters: Arc::new(filters),
        }
    }
}

#[async_trait::async_trait]
impl ProxyHttp for CardinalProxy {
    type CTX = Option<Arc<DestinationWrapper>>;

    fn new_ctx(&self) -> Self::CTX {
        None
    }

    async fn request_filter(&self, session: &mut Session, ctx: &mut Self::CTX) -> Result<bool> {
        let path = session.req_header().uri.path().to_string();
        info!(%path, "Request received");
        let destination_container = self
            .context
            .get::<DestinationContainer>()
            .await
            .map_err(|_| Error::new_str("Destination Container is not present"))?;

        let force_path = self.context.config.server.force_path_parameter;

        // Resolve backend or 404
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

        // Prepare upstream Host/SNI headers
        let _ = set_upstream_host_headers(session, &backend);
        info!(backend_id = %destination_name, "Routing to backend");

        // Set backend context and rewrite path
        *ctx = Some(backend.clone());
        rewrite_request_path(session.req_header_mut(), &destination_name, force_path);

        let run_filters = self.filters.run_request_filters(session, backend).await;

        let res = match run_filters {
            Ok(filter_result) => filter_result,
            Err(err) => {
                error!(%err, "Error running request filters");
                let _ = session.respond_error(500).await;
                return Ok(true);
            }
        };

        match res {
            FilterResult::Continue => {}
            FilterResult::Responded => return Ok(true),
        }

        Ok(false)
    }

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        if let Some(backend) = ctx {
            // Determine origin parts for TLS and SNI
            let (host, port, is_tls) = parse_origin(&backend.destination.url)
                .map_err(|_| Error::new_str("Origin could not be parsed "))?;
            let hostport = format!("{}:{}", host, port);

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
                // Prefer HTTP/2 with fallback to HTTP/1.1
                opts.set_http_version(2, 2);
            }
            let peer = Box::new(peer);
            Ok(peer)
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
        if let Some(backend) = ctx.as_ref() {
            self.filters
                .run_response_filters(session, backend.clone(), upstream_response)
                .await;
        }

        if !self.context.config.server.log_upstream_response {
            return Ok(());
        }

        let status = upstream_response.status.as_u16();
        let location = upstream_response
            .headers
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let backend_id = ctx
            .as_ref()
            .map(|b| b.destination.name.as_str())
            .unwrap_or("<unknown>");
        if let Some(loc) = location {
            info!(backend_id, status, location = %loc, "Upstream responded");
        } else {
            info!(backend_id, status, "Upstream responded");
        }

        Ok(())
    }
}
