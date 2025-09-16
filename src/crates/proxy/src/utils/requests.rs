use cardinal_errors::proxy::CardinalProxyError;
use http::Uri;
use pingora::http::RequestHeader;
use tracing::debug;

fn rewrite_request_path(req: &mut RequestHeader, backend_id: &str, force_path: bool) {
    if !force_path {
        return;
    }

    if let Some(pq) = req.uri.path_and_query() {
        let pq_str = pq.as_str();
        let (path_part, query_part) = match pq_str.split_once('?') {
            Some((p, q)) => (p, Some(q)),
            None => (pq_str, None),
        };

        if let Some(stripped) = path_part.strip_prefix(&format!("/{}", backend_id)) {
            let new_path = if stripped.is_empty() { "/" } else { stripped };
            let new_pq = match query_part {
                Some(q) if !q.is_empty() => format!("{}?{}", new_path, q),
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

fn parse_origin(origin: &str) -> Result<(String, u16, bool), CardinalProxyError> {
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
