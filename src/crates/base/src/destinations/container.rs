use crate::context::CardinalContext;
use crate::provider::Provider;
use crate::router::CardinalRouter;
use async_trait::async_trait;
use cardinal_config::{Destination, Middleware, MiddlewareType};
use cardinal_errors::CardinalError;
use pingora::http::RequestHeader;
use std::collections::BTreeMap;
use std::sync::Arc;

pub struct DestinationWrapper {
    pub destination: Destination,
    pub router: CardinalRouter,
    pub has_routes: bool,
    inbound_middleware: Vec<Middleware>,
    outbound_middleware: Vec<Middleware>,
}

impl DestinationWrapper {
    pub fn new(destination: Destination, router: Option<CardinalRouter>) -> Self {
        let inbound_middleware = destination
            .middleware
            .iter()
            .filter(|&e| e.r#type == MiddlewareType::Inbound)
            .cloned()
            .collect();
        let outbound_middleware = destination
            .middleware
            .iter()
            .filter(|&e| e.r#type == MiddlewareType::Outbound)
            .cloned()
            .collect();

        Self {
            has_routes: !destination.routes.is_empty(),
            destination,
            router: router.unwrap_or_default(),
            inbound_middleware,
            outbound_middleware,
        }
    }

    pub fn get_inbound_middleware(&self) -> &Vec<Middleware> {
        &self.inbound_middleware
    }

    pub fn get_outbound_middleware(&self) -> &Vec<Middleware> {
        &self.outbound_middleware
    }
}

pub struct DestinationContainer {
    destinations: BTreeMap<String, Arc<DestinationWrapper>>,
    default_destination: Option<Arc<DestinationWrapper>>,
}

impl DestinationContainer {
    pub fn get_backend_for_request(
        &self,
        req: &RequestHeader,
        force_parameter: bool,
    ) -> Option<Arc<DestinationWrapper>> {
        let candidate_id = if force_parameter {
            first_path_segment(req)
        } else {
            extract_subdomain(req)
        }?;

        self.destinations
            .get(&candidate_id)
            .cloned()
            .or_else(|| self.default_destination.clone())
    }
}

#[async_trait]
impl Provider for DestinationContainer {
    async fn provide(ctx: &CardinalContext) -> Result<Self, CardinalError> {
        let (destinations, default_destination) = ctx.config.destinations.clone().into_iter().fold(
            (BTreeMap::new(), None),
            |(mut map, default), (key, destination)| {
                let is_default = destination.default;
                let wrapper = Arc::new(DestinationWrapper::new(destination, None));
                let default = if is_default {
                    Some(wrapper.clone())
                } else {
                    default
                };

                map.insert(key, wrapper);
                (map, default)
            },
        );

        Ok(Self {
            destinations,
            default_destination,
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
