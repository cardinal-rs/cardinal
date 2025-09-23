mod utils;

use crate::utils::requests::{
    compose_upstream_url, parse_origin, rewrite_request_path, set_upstream_host_headers,
};
use cardinal_base::context::CardinalContext;
use cardinal_base::destinations::container::DestinationContainer;
use cardinal_config::Destination;
use pingora::http::ResponseHeader;
use pingora::prelude::*;
use pingora::protocols::Digest;
use pingora::upstreams::peer::Peer;
use std::sync::Arc;
use tracing::{debug, info, warn};

pub struct CardinalProxy {
    context: Arc<CardinalContext>,
}

impl CardinalProxy {
    pub fn new(context: Arc<CardinalContext>) -> Self {
        Self { context }
    }
}

#[async_trait::async_trait]
impl ProxyHttp for CardinalProxy {
    type CTX = Option<Destination>;

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

        // Prepare upstream Host/SNI headers
        let _ = set_upstream_host_headers(session, &backend);
        info!(backend_id = %backend.name, "Routing to backend");

        // Set backend context and rewrite path
        *ctx = Some(backend.clone());
        rewrite_request_path(session.req_header_mut(), &backend.name, force_path);

        Ok(false)
    }

    async fn upstream_peer(
        &self,
        _session: &mut Session,
        ctx: &mut Self::CTX,
    ) -> Result<Box<HttpPeer>> {
        if let Some(backend) = ctx {
            // Determine origin parts for TLS and SNI
            let (host, port, is_tls) = parse_origin(&backend.url)
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

            info!(%upstream_url, backend_id = %backend.name, is_tls, sni = %host, "Forwarding to upstream");
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
        let backend_id = ctx.as_ref().map(|b| b.name.as_str()).unwrap_or("<unknown>");
        info!(backend_id, reused, peer = %peer, "Connected to upstream");
        Ok(())
    }

    fn upstream_response_filter(
        &self,
        _session: &mut Session,
        upstream_response: &mut ResponseHeader,
        ctx: &mut Self::CTX,
    ) {
        if !self.context.config.log_upstream_response {
            return;
        }
        let status = upstream_response.status.as_u16();
        let location = upstream_response
            .headers
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let backend_id = ctx.as_ref().map(|b| b.name.as_str()).unwrap_or("<unknown>");
        if let Some(loc) = location {
            info!(backend_id, status, location = %loc, "Upstream responded");
        } else {
            info!(backend_id, status, "Upstream responded");
        }
    }
}
