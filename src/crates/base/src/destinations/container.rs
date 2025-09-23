use crate::context::CardinalContext;
use crate::provider::Provider;
use async_trait::async_trait;
use cardinal_config::Destination;
use cardinal_errors::CardinalError;
use pingora::http::RequestHeader;
use std::collections::BTreeMap;

pub struct DestinationContainer {
    destinations: BTreeMap<String, Destination>,
}

impl DestinationContainer {
    pub fn get_backend_for_request(
        &self,
        req: &RequestHeader,
        force_parameter: bool,
    ) -> Option<&Destination> {
        let candidate_id = if !force_parameter {
            extract_subdomain(req)
        } else {
            first_path_segment(req)
        };

        self.destinations.get(&candidate_id?)
    }
}

#[async_trait]
impl Provider for DestinationContainer {
    async fn provide(ctx: &CardinalContext) -> Result<Self, CardinalError> {
        Ok(Self {
            destinations: ctx.config.destinations.clone(),
        })
    }
}

fn first_path_segment(req: &RequestHeader) -> Option<String> {
    let path = req.uri.path();
    path.strip_prefix('/')
        .and_then(|p| p.split('/').next())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
}

fn extract_subdomain(req: &RequestHeader) -> Option<String> {
    let host = req.uri.host().map(|h| h.to_string()).or_else(|| {
        req.headers
            .get("host")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    })?;

    let host_no_port = host.split(':').next()?.to_ascii_lowercase();

    // Only treat as valid when there is a true subdomain: at least sub.domain.tld
    let parts: Vec<&str> = host_no_port.split('.').collect();
    if parts.len() < 3 {
        return None;
    }

    let first = parts[0];
    if first.is_empty() || first == "www" {
        None
    } else {
        Some(first.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::{Method, Uri};

    fn req_with_path(pq: &str) -> RequestHeader {
        RequestHeader::build(Method::GET, pq.as_bytes(), None).unwrap()
    }

    #[test]
    fn first_segment_basic() {
        let req = req_with_path("/api/users");
        assert_eq!(first_path_segment(&req), Some("api".to_string()));
    }

    #[test]
    fn first_segment_root_none() {
        let req = req_with_path("/");
        assert_eq!(first_path_segment(&req), None);
    }

    #[test]
    fn first_segment_case_insensitive() {
        let req = req_with_path("/API/v1");
        assert_eq!(first_path_segment(&req), Some("api".to_string()));
    }

    #[test]
    fn first_segment_trailing_slash() {
        let req = req_with_path("/api/");
        assert_eq!(first_path_segment(&req), Some("api".to_string()));
    }

    fn req_with_host_header(host: &str, path: &str) -> RequestHeader {
        let mut req = req_with_path(path);
        req.insert_header("host", host).unwrap();
        req
    }

    #[test]
    fn subdomain_from_host_header_basic() {
        let req = req_with_host_header("api.mygateway.com", "/any");
        assert_eq!(extract_subdomain(&req), Some("api".to_string()));
    }

    #[test]
    fn subdomain_from_host_header_with_port() {
        let req = req_with_host_header("api.mygateway.com:8080", "/any");
        assert_eq!(extract_subdomain(&req), Some("api".to_string()));
    }

    #[test]
    fn subdomain_www_is_ignored() {
        let req = req_with_host_header("www.mygateway.com", "/any");
        assert_eq!(extract_subdomain(&req), None);
    }

    #[test]
    fn subdomain_requires_at_least_domain_and_tld() {
        let req = req_with_host_header("localhost", "/any");
        assert_eq!(extract_subdomain(&req), None);
    }

    #[test]
    fn apex_domain_returns_none() {
        let req = req_with_host_header("mygateway.com", "/any");
        assert_eq!(extract_subdomain(&req), None);
    }

    #[test]
    fn subdomain_from_uri_authority() {
        let mut req = req_with_path("/any");
        let uri: Uri = "http://API.Example.com/any".parse().unwrap();
        req.set_uri(uri);
        assert_eq!(extract_subdomain(&req), Some("api".to_string()));
    }
}
