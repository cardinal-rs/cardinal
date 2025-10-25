use cardinal_base::destinations::container::DestinationWrapper;
use cardinal_errors::proxy::CardinalProxyError;
use cardinal_errors::CardinalError;
use cardinal_plugins::utils::parse_query_string_multi;
use cardinal_wasm_plugins::{ExecutionContext, ResponseState};
use http::Uri;
use parking_lot::RwLock;
use pingora::http::RequestHeader;
use pingora::proxy::Session;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::debug;

pub(crate) fn rewrite_request_path(req: &mut RequestHeader, backend_id: &str, force_path: bool) {
    if !force_path {
        return;
    }

    if let Some(pq) = req.uri.path_and_query() {
        let pq_str = pq.as_str();
        let (path_part, query_part) = match pq_str.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (pq_str, None),
        };

        if let Some(stripped) = path_part.strip_prefix(&format!("/{backend_id}")) {
            let new_path = if stripped.is_empty() { "/" } else { stripped };
            let new_pq = match query_part {
                Some(q) if !q.is_empty() => format!("{new_path}?{q}"),
                _ => new_path.to_string(),
            };

            let uri = Uri::builder()
                .path_and_query(new_pq.as_str())
                .build()
                .unwrap();
            debug!(%uri, "Rewrote downstream request path");
            req.set_uri(uri);
        }
    }
}

pub(crate) fn parse_origin(origin: &str) -> Result<(String, u16, bool), CardinalProxyError> {
    // Always give Uri a scheme; default to http:// if missing
    let origin_with_scheme = if origin.starts_with("http://") || origin.starts_with("https://") {
        origin.to_string()
    } else {
        format!("http://{origin}")
    };

    let uri: Uri = origin_with_scheme
        .parse()
        .map_err(|_| CardinalProxyError::BadUrl(origin_with_scheme.clone()))?;

    let is_tls = matches!(uri.scheme_str(), Some("https"));

    let auth = uri
        .authority()
        .ok_or(CardinalProxyError::BadUrl(uri.to_string()))?;

    let host = auth.host().to_string();
    let port = auth.port_u16().unwrap_or(if is_tls { 443 } else { 80 });

    Ok((host, port, is_tls))
}

pub(crate) fn compose_upstream_url(
    is_tls: bool,
    host: &str,
    port: u16,
    path_and_query: &str,
) -> String {
    let scheme = if is_tls { "https" } else { "http" };
    let mut hostport = host.to_string();
    if (is_tls && port != 443) || (!is_tls && port != 80) {
        hostport = format!("{host}:{port}");
    }
    let pq = if path_and_query.starts_with('/') {
        path_and_query.to_string()
    } else {
        format!("/{path_and_query}")
    };
    format!("{scheme}://{hostport}{pq}")
}

pub(crate) fn set_upstream_host_headers(
    session: &mut Session,
    backend: &Arc<DestinationWrapper>,
) -> Result<(), CardinalError> {
    let (up_host, up_port, up_tls) = parse_origin(&backend.destination.url)?;
    let header_host = if (up_tls && up_port == 443) || (!up_tls && up_port == 80) {
        up_host.clone()
    } else {
        format!("{up_host}:{up_port}")
    };

    // Preserve original Host
    let orig_host = session
        .req_header()
        .headers
        .get("Host")
        .and_then(|h| h.to_str().ok())
        .map(|s| s.to_string());

    // Set Host to upstream host for virtual hosting and TLS SNI
    session
        .req_header_mut()
        .insert_header("Host", header_host)
        .unwrap();

    if let Some(h) = orig_host {
        let _ = session
            .req_header_mut()
            .insert_header("X-Forwarded-Host", h);
    }

    Ok(())
}

pub(crate) fn execution_context_from_request(session: &Session) -> ExecutionContext {
    let get_req_headers = session.req_header().headers.clone();

    let query = parse_query_string_multi(session.req_header().uri.query().unwrap_or(""));

    ExecutionContext::from_parts(
        get_req_headers,
        query,
        None,
        ResponseState::with_default_status(200),
        Arc::new(RwLock::new(HashMap::new())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::Method;

    fn build_req(pq: &str) -> RequestHeader {
        RequestHeader::build(Method::GET, pq.as_bytes(), None).unwrap()
    }

    #[test]
    fn force_path_strips_prefix_without_query() {
        let mut req = build_req("/api/foo");
        rewrite_request_path(&mut req, "api", true);
        assert_eq!(req.uri.path_and_query().unwrap().as_str(), "/foo");
    }

    #[test]
    fn force_path_strips_prefix_root_without_query() {
        let mut req = build_req("/api");
        rewrite_request_path(&mut req, "api", true);
        assert_eq!(req.uri.path_and_query().unwrap().as_str(), "/");
    }

    #[test]
    fn force_path_strips_prefix_with_query() {
        let mut req = build_req("/api/foo?x=1&y=2");
        rewrite_request_path(&mut req, "api", true);
        assert_eq!(req.uri.path_and_query().unwrap().as_str(), "/foo?x=1&y=2");
    }

    #[test]
    fn force_path_strips_prefix_root_with_query() {
        let mut req = build_req("/api?x=1");
        rewrite_request_path(&mut req, "api", true);
        assert_eq!(req.uri.path_and_query().unwrap().as_str(), "/?x=1");
    }

    #[test]
    fn force_path_no_change_when_prefix_missing() {
        let original = "/v1/foo?bar=baz";
        let mut req = build_req(original);
        rewrite_request_path(&mut req, "api", true);
        assert_eq!(req.uri.path_and_query().unwrap().as_str(), original);
    }

    #[test]
    fn subdomain_mode_does_not_modify_path() {
        let original = "/api/foo?x=1";
        let mut req = build_req(original);
        rewrite_request_path(&mut req, "api", false);
        assert_eq!(req.uri.path_and_query().unwrap().as_str(), original);
    }

    // --- parse_origin tests ---
    #[test]
    fn parse_origin_http_default_port() {
        let (host, port, tls) = parse_origin("http://example.com").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 80);
        assert!(!tls);
    }

    #[test]
    fn parse_origin_https_default_port() {
        let (host, port, tls) = parse_origin("https://example.com").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 443);
        assert!(tls);
    }

    #[test]
    fn parse_origin_with_explicit_port() {
        let (host, port, tls) = parse_origin("https://example.com:8443").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 8443);
        assert!(tls);
    }

    #[test]
    fn parse_origin_without_scheme_defaults_http() {
        let (host, port, tls) = parse_origin("example.com:8080").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 8080);
        assert!(!tls);
    }

    #[test]
    fn parse_origin_ignores_path_after_authority() {
        let (host, port, tls) = parse_origin("http://example.com/foo/bar").unwrap();
        assert_eq!(host, "example.com");
        assert_eq!(port, 80);
        assert!(!tls);
    }

    #[test]
    fn parse_origin_invalid_url_errors() {
        match parse_origin("http://") {
            Err(CardinalProxyError::BadUrl(s)) => assert!(s.contains("http://")),
            other => panic!("Expected BadUrl error, got: {:?}", other),
        }
    }

    #[test]
    fn parse_origin_ipv6_host_with_port() {
        let (host, port, tls) = parse_origin("http://[::1]:8080").unwrap();
        assert_eq!(host, "[::1]");
        assert_eq!(port, 8080);
        assert!(!tls);
    }
}
