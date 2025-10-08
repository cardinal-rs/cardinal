use std::collections::HashMap;
use std::sync::Arc;

use cardinal_config::{DestinationMatch, DestinationMatchValue};
use cardinal_errors::CardinalError;
use pingora::http::RequestHeader;
use regex::Regex;

use crate::destinations::container::DestinationWrapper;

pub struct DestinationMatcherIndex {
    exact_host: HashMap<String, Vec<CompiledDestination>>,
    regex_host: Vec<RegexHostEntry>,
    hostless: Vec<CompiledDestination>,
}

impl DestinationMatcherIndex {
    pub fn new(
        destinations: impl Iterator<Item = Arc<DestinationWrapper>>,
    ) -> Result<Self, CardinalError> {
        let mut exact_host: HashMap<String, Vec<CompiledDestination>> = HashMap::new();
        let mut regex_host: Vec<RegexHostEntry> = Vec::new();
        let mut hostless: Vec<CompiledDestination> = Vec::new();

        for wrapper in destinations {
            let Some(matchers) = wrapper.destination.r#match.as_ref() else {
                continue;
            };

            if matchers.is_empty() {
                continue;
            }

            for matcher in matchers {
                let compiled = CompiledEntry::try_from(wrapper.clone(), matcher)?;
                match compiled.host_matcher {
                    Some(CompiledHostMatcher::Exact(host)) => {
                        exact_host
                            .entry(host)
                            .or_default()
                            .push(compiled.destination);
                    }
                    Some(CompiledHostMatcher::Regex(regex)) => {
                        regex_host.push(RegexHostEntry {
                            matcher: regex,
                            destination: compiled.destination,
                        });
                    }
                    None => hostless.push(compiled.destination),
                }
            }
        }

        Ok(Self {
            exact_host,
            regex_host,
            hostless,
        })
    }

    pub fn resolve(&self, req: &RequestHeader) -> Option<Arc<DestinationWrapper>> {
        let host = request_host(req);
        let path = req.uri.path();

        if let Some(host) = host.as_deref() {
            if let Some(entries) = self.exact_host.get(host) {
                // Exact host matches can still vary by path (e.g. /billing vs /support).
                // Walk the candidates and keep the first whose path rules apply.
                if let Some(wrapper) = entries
                    .iter()
                    .find_map(|destination| destination.matches(path))
                {
                    return Some(wrapper);
                }
            }

            for entry in &self.regex_host {
                if entry.matcher.is_match(host) {
                    if let Some(wrapper) = entry.destination.matches(path) {
                        return Some(wrapper);
                    }
                }
            }
        }

        for destination in &self.hostless {
            if let Some(wrapper) = destination.matches(path) {
                return Some(wrapper);
            }
        }

        None
    }
}

struct RegexHostEntry {
    matcher: Regex,
    destination: CompiledDestination,
}

struct CompiledEntry {
    host_matcher: Option<CompiledHostMatcher>,
    destination: CompiledDestination,
}

impl CompiledEntry {
    fn try_from(
        wrapper: Arc<DestinationWrapper>,
        matcher: &DestinationMatch,
    ) -> Result<Self, CardinalError> {
        let host_matcher = compile_host_matcher(matcher.host.as_ref())?;
        let path_prefix = compile_path_prefix(matcher.path_prefix.as_ref())?;
        let path_exact = matcher.path_exact.clone();

        let destination = CompiledDestination {
            wrapper,
            path_prefix,
            path_exact,
        };

        Ok(Self {
            host_matcher,
            destination,
        })
    }
}

enum CompiledHostMatcher {
    Exact(String),
    Regex(Regex),
}

struct CompiledDestination {
    wrapper: Arc<DestinationWrapper>,
    path_prefix: Option<CompiledPathMatcher>,
    path_exact: Option<String>,
}

impl CompiledDestination {
    fn matches(&self, path: &str) -> Option<Arc<DestinationWrapper>> {
        if self.matches_path(path) {
            Some(self.wrapper.clone())
        } else {
            None
        }
    }

    fn matches_path(&self, path: &str) -> bool {
        if let Some(exact) = &self.path_exact {
            if path != exact {
                return false;
            }
        }

        if let Some(prefix) = &self.path_prefix {
            return prefix.matches(path);
        }

        true
    }
}

enum CompiledPathMatcher {
    Prefix(String),
    Regex(Regex),
}

impl CompiledPathMatcher {
    fn matches(&self, path: &str) -> bool {
        match self {
            CompiledPathMatcher::Prefix(prefix) => path.starts_with(prefix),
            CompiledPathMatcher::Regex(regex) => regex.is_match(path),
        }
    }
}

fn compile_host_matcher(
    value: Option<&DestinationMatchValue>,
) -> Result<Option<CompiledHostMatcher>, CardinalError> {
    match value {
        Some(DestinationMatchValue::String(host)) => {
            Ok(Some(CompiledHostMatcher::Exact(host.to_ascii_lowercase())))
        }
        Some(DestinationMatchValue::Regex { regex }) => {
            let compiled = Regex::new(regex).map_err(|err| {
                CardinalError::Other(format!("invalid host regex '{regex}': {err}"))
            })?;
            Ok(Some(CompiledHostMatcher::Regex(compiled)))
        }
        None => Ok(None),
    }
}

fn compile_path_prefix(
    value: Option<&DestinationMatchValue>,
) -> Result<Option<CompiledPathMatcher>, CardinalError> {
    match value {
        Some(DestinationMatchValue::String(prefix)) => {
            Ok(Some(CompiledPathMatcher::Prefix(prefix.clone())))
        }
        Some(DestinationMatchValue::Regex { regex }) => {
            let compiled = Regex::new(regex).map_err(|err| {
                CardinalError::Other(format!("invalid path regex '{regex}': {err}"))
            })?;
            Ok(Some(CompiledPathMatcher::Regex(compiled)))
        }
        None => Ok(None),
    }
}

fn request_host(req: &RequestHeader) -> Option<String> {
    let host = req.uri.host().map(|h| h.to_string()).or_else(|| {
        req.headers
            .get("host")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    })?;

    let host_no_port = host.split(':').next()?.to_ascii_lowercase();
    Some(host_no_port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cardinal_config::{Destination, DestinationMatch};
    use http::Method;
    use pingora::http::RequestHeader;

    fn build_destination(
        name: &str,
        host: Option<DestinationMatchValue>,
        path_prefix: Option<DestinationMatchValue>,
        path_exact: Option<&str>,
    ) -> Arc<DestinationWrapper> {
        build_destination_with_matchers(
            name,
            Some(vec![DestinationMatch {
                host,
                path_prefix,
                path_exact: path_exact.map(|s| s.to_string()),
            }]),
        )
    }

    fn build_destination_with_matchers(
        name: &str,
        matchers: Option<Vec<DestinationMatch>>,
    ) -> Arc<DestinationWrapper> {
        let destination = Destination {
            name: name.to_string(),
            url: "https://example.com".to_string(),
            health_check: None,
            default: false,
            r#match: matchers,
            routes: Vec::new(),
            middleware: Vec::new(),
        };

        Arc::new(DestinationWrapper::new(destination, None))
    }

    fn build_request(host: &str, path: &str) -> RequestHeader {
        let mut req = RequestHeader::build(Method::GET, path.as_bytes(), None).unwrap();
        req.insert_header("host", host).unwrap();
        req
    }

    #[test]
    fn matches_exact_host() {
        let destination = build_destination(
            "customer_service",
            Some(DestinationMatchValue::String("api.example.com".into())),
            None,
            None,
        );

        let matcher = DestinationMatcherIndex::new(vec![destination.clone()].into_iter()).unwrap();
        let req = build_request("API.EXAMPLE.com", "/v1/customers");

        let resolved = matcher.resolve(&req).unwrap();
        assert_eq!(resolved.destination.name, "customer_service");
    }

    #[test]
    fn matches_host_regex() {
        let destination = build_destination(
            "billing",
            Some(DestinationMatchValue::Regex {
                regex: "^api\\.(eu|us)\\.example\\.com$".into(),
            }),
            None,
            None,
        );

        let matcher = DestinationMatcherIndex::new(vec![destination.clone()].into_iter()).unwrap();
        let req = build_request("api.eu.example.com", "/billing");

        let resolved = matcher.resolve(&req).unwrap();
        assert_eq!(resolved.destination.name, "billing");
    }

    #[test]
    fn supports_multiple_match_entries_per_destination() {
        let destination = build_destination_with_matchers(
            "api",
            Some(vec![
                DestinationMatch {
                    host: Some(DestinationMatchValue::String("api.example.com".into())),
                    path_prefix: Some(DestinationMatchValue::String("/billing".into())),
                    path_exact: None,
                },
                DestinationMatch {
                    host: Some(DestinationMatchValue::String("api.example.com".into())),
                    path_prefix: Some(DestinationMatchValue::String("/support".into())),
                    path_exact: None,
                },
            ]),
        );

        let matcher = DestinationMatcherIndex::new(vec![destination.clone()].into_iter()).unwrap();

        let billing_req = build_request("api.example.com", "/billing/payments");
        let billing_destination = matcher.resolve(&billing_req).unwrap();
        assert_eq!(billing_destination.destination.name, "api");

        let support_req = build_request("api.example.com", "/support/chat");
        let support_destination = matcher.resolve(&support_req).unwrap();
        assert_eq!(support_destination.destination.name, "api");

        let missing_req = build_request("api.example.com", "/reports");
        assert!(matcher.resolve(&missing_req).is_none());
    }

    #[test]
    fn exact_host_entries_are_prioritized_before_regex() {
        let destination = build_destination_with_matchers(
            "api",
            Some(vec![
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
        );

        let matcher = DestinationMatcherIndex::new(vec![destination.clone()].into_iter()).unwrap();

        let exact_req = build_request("api.example.com", "/billing/invoices");
        let exact_destination = matcher.resolve(&exact_req).unwrap();
        assert_eq!(exact_destination.destination.name, "api");

        let regex_req = build_request("api.example.com", "/regex/search");
        let regex_destination = matcher.resolve(&regex_req).unwrap();
        assert_eq!(regex_destination.destination.name, "api");
    }

    #[test]
    fn matches_path_prefix() {
        let hostless = build_destination(
            "helpdesk",
            None,
            Some(DestinationMatchValue::String("/helpdesk".into())),
            None,
        );

        let matcher = DestinationMatcherIndex::new(vec![hostless.clone()].into_iter()).unwrap();
        let req = build_request("any.example.com", "/helpdesk/ticket");

        let resolved = matcher.resolve(&req).unwrap();
        assert_eq!(resolved.destination.name, "helpdesk");
    }

    #[test]
    fn matches_path_regex() {
        let hostless = build_destination(
            "reports",
            None,
            Some(DestinationMatchValue::Regex {
                regex: "^/reports/(daily|weekly)".into(),
            }),
            None,
        );

        let matcher = DestinationMatcherIndex::new(vec![hostless.clone()].into_iter()).unwrap();
        let req = build_request("other.example.com", "/reports/daily/summary");

        let resolved = matcher.resolve(&req).unwrap();
        assert_eq!(resolved.destination.name, "reports");
    }

    #[test]
    fn respects_path_exact() {
        let host = build_destination(
            "status",
            Some(DestinationMatchValue::String("status.example.com".into())),
            None,
            Some("/healthz"),
        );

        let matcher = DestinationMatcherIndex::new(vec![host.clone()].into_iter()).unwrap();
        let req = build_request("status.example.com", "/healthz");

        assert!(matcher.resolve(&req).is_some());

        let req_non_matching = build_request("status.example.com", "/healthz/extra");
        assert!(matcher.resolve(&req_non_matching).is_none());
    }

    #[test]
    fn host_priority_before_hostless() {
        let host_destination = build_destination(
            "api",
            Some(DestinationMatchValue::String("api.example.com".into())),
            None,
            None,
        );
        let hostless = build_destination("fallback", None, None, None);

        let matcher = DestinationMatcherIndex::new(
            vec![hostless.clone(), host_destination.clone()].into_iter(),
        )
        .unwrap();
        let req = build_request("api.example.com", "/anything");

        let resolved = matcher.resolve(&req).unwrap();
        assert_eq!(resolved.destination.name, "api");
    }
}
