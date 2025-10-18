use crate::context::CardinalContext;
use crate::destinations::matcher::DestinationMatcherIndex;
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
    matcher: DestinationMatcherIndex,
}

impl DestinationContainer {
    pub fn get_backend_for_request(
        &self,
        req: &RequestHeader,
        force_parameter: bool,
    ) -> Option<Arc<DestinationWrapper>> {
        let matcher_hit = if force_parameter {
            None
        } else {
            self.matcher.resolve(req)
        };

        matcher_hit.or_else(|| {
            let candidate = if force_parameter {
                first_path_segment(req)
            } else {
                extract_subdomain(req)
            };

            candidate
                .and_then(|key| self.destinations.get(&key).cloned())
                .or_else(|| self.default_destination.clone())
        })
    }
}

#[async_trait]
impl Provider for DestinationContainer {
    async fn provide(ctx: &CardinalContext) -> Result<Self, CardinalError> {
        let mut destinations: BTreeMap<String, Arc<DestinationWrapper>> = BTreeMap::new();
        let mut default_destination = None;
        let mut wrappers: Vec<Arc<DestinationWrapper>> = Vec::new();

        for (key, destination) in ctx.config.destinations.clone() {
            let has_match = destination
                .r#match
                .as_ref()
                .map(|entries| !entries.is_empty())
                .unwrap_or(false);
            let router = destination
                .routes
                .iter()
                .fold(CardinalRouter::new(), |mut r, route| {
                    let _ = r.add(route.method.as_str(), route.path.as_str());
                    r
                });

            let wrapper = Arc::new(DestinationWrapper::new(destination, Some(router)));

            if wrapper.destination.default {
                default_destination = Some(wrapper.clone());
            }

            if !has_match {
                destinations.insert(key, Arc::clone(&wrapper));
            }
            // Every destination participates in the matcher, even if it also lives in the
            // legacy map (for matcher-less configs).
            wrappers.push(wrapper);
        }

        let matcher = DestinationMatcherIndex::new(wrappers.into_iter())?;

        Ok(Self {
            destinations,
            default_destination,
            matcher,
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
    use cardinal_config::{Destination, DestinationMatch, DestinationMatchValue};
    use http::{Method, Uri};
    use std::collections::BTreeMap;
    use std::sync::Arc;

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

    fn destination_config(
        name: &str,
        host: Option<DestinationMatchValue>,
        path_prefix: Option<DestinationMatchValue>,
        path_exact: Option<&str>,
        default: bool,
    ) -> Destination {
        Destination {
            name: name.to_string(),
            url: format!("https://{name}.internal"),
            health_check: None,
            default,
            r#match: Some(vec![DestinationMatch {
                host,
                path_prefix,
                path_exact: path_exact.map(|s| s.to_string()),
            }]),
            routes: Vec::new(),
            middleware: Vec::new(),
            timeout: None,
            retry: None,
        }
    }

    fn build_container(entries: Vec<(&str, Destination)>) -> DestinationContainer {
        let mut destinations = BTreeMap::new();
        let mut default_destination = None;
        let mut wrappers = Vec::new();

        for (key, destination) in entries {
            let has_match = destination
                .r#match
                .as_ref()
                .map(|entries| !entries.is_empty())
                .unwrap_or(false);
            let wrapper = Arc::new(DestinationWrapper::new(destination, None));
            if wrapper.destination.default {
                default_destination = Some(wrapper.clone());
            }
            if !has_match {
                destinations.insert(key.to_string(), Arc::clone(&wrapper));
            }
            // The matcher should see every destination regardless of legacy eligibility.
            wrappers.push(wrapper);
        }

        let matcher = DestinationMatcherIndex::new(wrappers.into_iter()).unwrap();

        DestinationContainer {
            destinations,
            default_destination,
            matcher,
        }
    }

    #[test]
    fn resolves_destination_by_host_match() {
        let container = build_container(vec![(
            "customer",
            destination_config(
                "customer",
                Some(DestinationMatchValue::String("support.example.com".into())),
                None,
                None,
                false,
            ),
        )]);

        let req = req_with_host_header("support.example.com", "/any");
        let resolved = container.get_backend_for_request(&req, false).unwrap();
        assert_eq!(resolved.destination.name, "customer");
    }

    #[test]
    fn resolves_destination_by_host_regex() {
        let container = build_container(vec![(
            "billing",
            destination_config(
                "billing",
                Some(DestinationMatchValue::Regex {
                    regex: "^api\\.(eu|us)\\.example\\.com$".into(),
                }),
                None,
                None,
                false,
            ),
        )]);

        let req = req_with_host_header("api.eu.example.com", "/billing/pay");
        let resolved = container.get_backend_for_request(&req, false).unwrap();
        assert_eq!(resolved.destination.name, "billing");
    }

    #[test]
    fn resolves_destination_by_path_prefix() {
        let container = build_container(vec![(
            "helpdesk",
            destination_config(
                "helpdesk",
                None,
                Some(DestinationMatchValue::String("/helpdesk".into())),
                None,
                false,
            ),
        )]);

        let req = req_with_host_header("any.example.com", "/helpdesk/ticket");
        let resolved = container.get_backend_for_request(&req, false).unwrap();
        assert_eq!(resolved.destination.name, "helpdesk");
    }

    #[test]
    fn falls_back_to_default_when_no_match() {
        let container = build_container(vec![(
            "primary",
            destination_config(
                "primary",
                Some(DestinationMatchValue::String("app.example.com".into())),
                None,
                None,
                true,
            ),
        )]);

        let req = req_with_host_header("unknown.example.com", "/unknown");
        let resolved = container.get_backend_for_request(&req, false).unwrap();
        assert_eq!(resolved.destination.name, "primary");
    }

    #[test]
    fn does_not_reintroduce_non_matching_host_rule() {
        let mut entries = Vec::new();
        entries.push((
            "billing",
            destination_config(
                "billing",
                Some(DestinationMatchValue::String("billing.example.com".into())),
                Some(DestinationMatchValue::String("/billing".into())),
                None,
                false,
            ),
        ));

        let default_destination = Destination {
            name: "fallback".into(),
            url: "https://fallback.internal".into(),
            health_check: None,
            default: true,
            r#match: None,
            routes: Vec::new(),
            middleware: Vec::new(),
            timeout: None,
            retry: None,
        };

        entries.push(("fallback", default_destination));

        let container = build_container(entries);
        let req = req_with_host_header("billing.example.com", "/other");

        let resolved = container.get_backend_for_request(&req, false).unwrap();
        assert_eq!(resolved.destination.name, "fallback");
    }

    #[test]
    fn selects_destination_among_shared_host_paths() {
        let container = build_container(vec![
            (
                "billing",
                destination_config(
                    "billing",
                    Some(DestinationMatchValue::String("api.example.com".into())),
                    Some(DestinationMatchValue::String("/billing".into())),
                    None,
                    false,
                ),
            ),
            (
                "support",
                destination_config(
                    "support",
                    Some(DestinationMatchValue::String("api.example.com".into())),
                    Some(DestinationMatchValue::String("/support".into())),
                    None,
                    false,
                ),
            ),
            (
                "fallback",
                Destination {
                    name: "fallback".into(),
                    url: "https://fallback.internal".into(),
                    health_check: None,
                    default: true,
                    r#match: None,
                    routes: Vec::new(),
                    middleware: Vec::new(),
                    timeout: None,
                    retry: None,
                },
            ),
        ]);

        let req = req_with_host_header("api.example.com", "/support/ticket");
        let resolved = container.get_backend_for_request(&req, false).unwrap();
        assert_eq!(resolved.destination.name, "support");
    }

    #[test]
    fn falls_back_when_shared_host_paths_do_not_match() {
        let container = build_container(vec![
            (
                "billing",
                destination_config(
                    "billing",
                    Some(DestinationMatchValue::String("api.example.com".into())),
                    Some(DestinationMatchValue::String("/billing".into())),
                    None,
                    false,
                ),
            ),
            (
                "fallback",
                Destination {
                    name: "fallback".into(),
                    url: "https://fallback.internal".into(),
                    health_check: None,
                    default: true,
                    r#match: None,
                    routes: Vec::new(),
                    middleware: Vec::new(),
                    timeout: None,
                    retry: None,
                },
            ),
        ]);

        let req = req_with_host_header("api.example.com", "/reports");
        let resolved = container.get_backend_for_request(&req, false).unwrap();
        assert_eq!(resolved.destination.name, "fallback");
    }

    #[test]
    fn host_regex_entries_consider_path_rules() {
        let container = build_container(vec![
            (
                "billing",
                destination_config(
                    "billing",
                    Some(DestinationMatchValue::Regex {
                        regex: "^api\\.(eu|us)\\.example\\.com$".into(),
                    }),
                    Some(DestinationMatchValue::String("/billing".into())),
                    None,
                    false,
                ),
            ),
            (
                "support",
                destination_config(
                    "support",
                    Some(DestinationMatchValue::Regex {
                        regex: "^api\\.(eu|us)\\.example\\.com$".into(),
                    }),
                    Some(DestinationMatchValue::String("/support".into())),
                    None,
                    false,
                ),
            ),
            (
                "fallback",
                Destination {
                    name: "fallback".into(),
                    url: "https://fallback.internal".into(),
                    health_check: None,
                    default: true,
                    r#match: None,
                    routes: Vec::new(),
                    middleware: Vec::new(),
                    timeout: None,
                    retry: None,
                },
            ),
        ]);

        let req = req_with_host_header("api.eu.example.com", "/support/chat");
        let resolved = container.get_backend_for_request(&req, false).unwrap();
        assert_eq!(resolved.destination.name, "support");
    }

    #[test]
    fn hostless_entries_respect_path_order() {
        let container = build_container(vec![
            (
                "reports",
                destination_config(
                    "reports",
                    None,
                    Some(DestinationMatchValue::Regex {
                        regex: "^/reports/(daily|weekly)".into(),
                    }),
                    None,
                    false,
                ),
            ),
            (
                "billing",
                destination_config(
                    "billing",
                    None,
                    Some(DestinationMatchValue::String("/billing".into())),
                    None,
                    false,
                ),
            ),
            (
                "fallback",
                Destination {
                    name: "fallback".into(),
                    url: "https://fallback.internal".into(),
                    health_check: None,
                    default: true,
                    r#match: None,
                    routes: Vec::new(),
                    middleware: Vec::new(),
                    timeout: None,
                    retry: None,
                },
            ),
        ]);

        let req_reports = req_with_host_header("any.example.com", "/reports/daily/summary");
        let resolved_reports = container
            .get_backend_for_request(&req_reports, false)
            .unwrap();
        assert_eq!(resolved_reports.destination.name, "reports");

        let req_billing = req_with_host_header("any.example.com", "/billing/invoice");
        let resolved_billing = container
            .get_backend_for_request(&req_billing, false)
            .unwrap();
        assert_eq!(resolved_billing.destination.name, "billing");

        let req_fallback = req_with_host_header("any.example.com", "/unknown");
        let resolved_fallback = container
            .get_backend_for_request(&req_fallback, false)
            .unwrap();
        assert_eq!(resolved_fallback.destination.name, "fallback");
    }

    #[test]
    fn force_parameter_ignores_match_enabled_destinations() {
        let container = build_container(vec![
            (
                "matched",
                destination_config(
                    "matched",
                    Some(DestinationMatchValue::String("api.example.com".into())),
                    Some(DestinationMatchValue::String("/matched".into())),
                    None,
                    false,
                ),
            ),
            (
                "fallback",
                Destination {
                    name: "fallback".into(),
                    url: "https://fallback.internal".into(),
                    health_check: None,
                    default: true,
                    r#match: None,
                    routes: Vec::new(),
                    middleware: Vec::new(),
                    timeout: None,
                    retry: None,
                },
            ),
        ]);

        let req = req_with_path("/matched/orders");
        let resolved = container.get_backend_for_request(&req, true).unwrap();
        assert_eq!(resolved.destination.name, "fallback");
    }

    #[test]
    fn path_exact_precedence_over_prefix() {
        let container = build_container(vec![
            (
                "status_exact",
                destination_config(
                    "status_exact",
                    Some(DestinationMatchValue::String("status.example.com".into())),
                    None,
                    Some("/status"),
                    false,
                ),
            ),
            (
                "status_prefix",
                destination_config(
                    "status_prefix",
                    Some(DestinationMatchValue::String("status.example.com".into())),
                    Some(DestinationMatchValue::String("/status".into())),
                    None,
                    false,
                ),
            ),
        ]);

        let req = req_with_host_header("status.example.com", "/status");
        let resolved = container.get_backend_for_request(&req, false).unwrap();
        assert_eq!(resolved.destination.name, "status_exact");
    }

    #[test]
    fn regex_host_prefers_matching_path_before_default() {
        let container = build_container(vec![
            (
                "v1",
                destination_config(
                    "v1",
                    Some(DestinationMatchValue::Regex {
                        regex: "^api\\.(eu|us)\\.example\\.com$".into(),
                    }),
                    Some(DestinationMatchValue::String("/v1".into())),
                    None,
                    false,
                ),
            ),
            (
                "v2",
                destination_config(
                    "v2",
                    Some(DestinationMatchValue::Regex {
                        regex: "^api\\.(eu|us)\\.example\\.com$".into(),
                    }),
                    Some(DestinationMatchValue::String("/v2".into())),
                    None,
                    false,
                ),
            ),
            (
                "fallback",
                Destination {
                    name: "fallback".into(),
                    url: "https://fallback.internal".into(),
                    health_check: None,
                    default: true,
                    r#match: None,
                    routes: Vec::new(),
                    middleware: Vec::new(),
                    timeout: None,
                    retry: None,
                },
            ),
        ]);

        let req_v2 = req_with_host_header("api.eu.example.com", "/v2/items");
        let resolved_v2 = container.get_backend_for_request(&req_v2, false).unwrap();
        assert_eq!(resolved_v2.destination.name, "v2");

        let req_none = req_with_host_header("api.eu.example.com", "/v3/unknown");
        let resolved_none = container.get_backend_for_request(&req_none, false).unwrap();
        assert_eq!(resolved_none.destination.name, "fallback");
    }

    #[test]
    fn hostless_entries_prioritize_config_order() {
        let container = build_container(vec![
            (
                "reports_regex",
                destination_config(
                    "reports_regex",
                    None,
                    Some(DestinationMatchValue::Regex {
                        regex: "^/reports/.*".into(),
                    }),
                    None,
                    false,
                ),
            ),
            (
                "reports_prefix",
                destination_config(
                    "reports_prefix",
                    None,
                    Some(DestinationMatchValue::String("/reports".into())),
                    None,
                    false,
                ),
            ),
        ]);

        let req = req_with_host_header("any.example.com", "/reports/daily");
        let resolved = container.get_backend_for_request(&req, false).unwrap();
        assert_eq!(resolved.destination.name, "reports_regex");
    }

    #[test]
    fn force_parameter_falls_back_to_default_when_unknown() {
        let container = build_container(vec![(
            "fallback",
            Destination {
                name: "fallback".into(),
                url: "https://fallback.internal".into(),
                health_check: None,
                default: true,
                r#match: None,
                routes: Vec::new(),
                middleware: Vec::new(),
                timeout: None,
                retry: None,
            },
        )]);

        let req = req_with_path("/unknown/path");
        let resolved = container.get_backend_for_request(&req, true).unwrap();
        assert_eq!(resolved.destination.name, "fallback");
    }

    #[test]
    fn returns_none_when_no_match_and_no_default() {
        let container = build_container(vec![(
            "matcher_only",
            destination_config(
                "matcher_only",
                Some(DestinationMatchValue::String("api.example.com".into())),
                Some(DestinationMatchValue::String("/matcher".into())),
                None,
                false,
            ),
        )]);

        let req = req_with_host_header("api.example.com", "/unknown");
        let resolved = container.get_backend_for_request(&req, false);
        assert!(resolved.is_none());
    }

    #[test]
    fn multi_match_destination_skips_legacy_map() {
        let destination = Destination {
            name: "shared".into(),
            url: "https://shared.internal".into(),
            health_check: None,
            default: false,
            r#match: Some(vec![
                DestinationMatch {
                    host: Some(DestinationMatchValue::String("api.example.com".into())),
                    path_prefix: Some(DestinationMatchValue::String("/billing".into())),
                    path_exact: None,
                },
                DestinationMatch {
                    host: Some(DestinationMatchValue::Regex {
                        regex: "^api\\..+".into(),
                    }),
                    path_prefix: Some(DestinationMatchValue::String("/regex".into())),
                    path_exact: None,
                },
            ]),
            routes: Vec::new(),
            middleware: Vec::new(),
            timeout: None,
            retry: None,
        };

        let container = build_container(vec![("shared", destination)]);

        assert!(container.destinations.get("shared").is_none());

        let exact_req = req_with_host_header("api.example.com", "/billing/invoices");
        let exact_resolved = container
            .get_backend_for_request(&exact_req, false)
            .unwrap();
        assert_eq!(exact_resolved.destination.name, "shared");

        let regex_req = req_with_host_header("api.example.com", "/regex/search");
        let regex_resolved = container
            .get_backend_for_request(&regex_req, false)
            .unwrap();
        assert_eq!(regex_resolved.destination.name, "shared");
    }

    #[test]
    fn force_parameter_uses_path_segment_lookup() {
        let destination = Destination {
            name: "segment".into(),
            url: "https://segment.internal".into(),
            health_check: None,
            default: false,
            r#match: None,
            routes: Vec::new(),
            middleware: Vec::new(),
            timeout: None,
            retry: None,
        };

        let container = build_container(vec![("segment", destination)]);
        let req = req_with_path("/segment/orders");

        let resolved = container.get_backend_for_request(&req, true).unwrap();
        assert_eq!(resolved.destination.name, "segment");
    }

    #[test]
    fn falls_back_to_subdomain_key_when_present() {
        let destination = Destination {
            name: "api".into(),
            url: "https://api.internal".into(),
            health_check: None,
            default: false,
            r#match: None,
            routes: Vec::new(),
            middleware: Vec::new(),
            timeout: None,
            retry: None,
        };

        let container = build_container(vec![("api", destination)]);
        let req = req_with_host_header("api.mygateway.com", "/any");

        let resolved = container.get_backend_for_request(&req, false).unwrap();
        assert_eq!(resolved.destination.name, "api");
    }
}
