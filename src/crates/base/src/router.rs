use cardinal_errors::internal::CardinalInternalError;
use cardinal_errors::CardinalError;
use matchit::Router;
use std::collections::HashMap;

pub struct CardinalRouter {
    router: Router<()>,
}

impl CardinalRouter {
    pub fn new() -> Self {
        Self {
            router: Router::new(),
        }
    }

    pub fn add(&mut self, method: &str, path: &str) -> Result<(), CardinalError> {
        self.router
            .insert(format!("{}:{}", method, path), ())
            .map_err(|e| CardinalInternalError::InvalidRouteConfiguration(e.to_string()))?;
        Ok(())
    }

    pub fn valid(&self, method: &str, path: &str) -> Option<(bool, HashMap<String, String>)> {
        let actual_path = format!("{}:{}", method, path);
        let route_res = self.router.at(actual_path.as_str());
        match route_res {
            Ok(e) => Some((
                true,
                e.params
                    .iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
            )),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_returns_some_for_registered_route() {
        let mut router = CardinalRouter::new();
        router.add("GET", "/status").unwrap();

        let (is_match, params) = router.valid("GET", "/status").expect("route should match");

        assert!(is_match);
        assert!(params.is_empty());
    }

    #[test]
    fn valid_extracts_path_params() {
        let mut router = CardinalRouter::new();
        router.add("GET", "/items/{id}/detail").unwrap();

        let (_, params) = router
            .valid("GET", "/items/123/detail")
            .expect("route should match");

        assert_eq!(params.len(), 1);
        assert_eq!(params.get("id").map(String::as_str), Some("123"));
    }

    #[test]
    fn valid_returns_none_for_unregistered_route() {
        let mut router = CardinalRouter::new();
        router.add("POST", "/submit").unwrap();

        assert!(router.valid("GET", "/submit").is_none());
        assert!(router.valid("POST", "/unknown").is_none());
    }

    #[test]
    fn valid_returns_none_when_path_param_missing() {
        let mut router = CardinalRouter::new();
        router.add("GET", "/items/{id}/detail").unwrap();

        assert!(router.valid("GET", "/items/detail").is_none());
        assert!(router.valid("GET", "/items//detail").is_none());
    }

    #[test]
    fn valid_returns_none_when_path_has_extra_segments() {
        let mut router = CardinalRouter::new();
        router.add("GET", "/items/{id}").unwrap();

        assert!(router.valid("GET", "/items/123/extra").is_none());
    }

    #[test]
    fn valid_is_none_when_method_differs_in_case() {
        let mut router = CardinalRouter::new();
        router.add("GET", "/status").unwrap();

        assert!(router.valid("get", "/status").is_none());
    }

    #[test]
    fn add_duplicate_route_returns_error() {
        let mut router = CardinalRouter::new();
        router.add("GET", "/status").unwrap();

        let err = router.add("GET", "/status").unwrap_err();

        match err {
            CardinalError::InternalError(CardinalInternalError::InvalidRouteConfiguration(_)) => {}
            _ => panic!("expected InvalidRouteConfiguration error"),
        }
    }
}
