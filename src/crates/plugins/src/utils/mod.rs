use std::collections::HashMap;

/// Parse query string into a `HashMap<String, Vec<String>>`
/// Keeps all values when a key appears multiple times.
pub fn parse_query_string_multi(qs: &str) -> HashMap<String, Vec<String>> {
    let mut map: HashMap<String, Vec<String>> = HashMap::new();

    for (key, value) in form_urlencoded::parse(qs.as_bytes()).into_owned() {
        map.entry(key).or_default().push(value);
    }

    map
}

#[cfg(test)]
mod tests {
    use super::parse_query_string_multi;
    use std::collections::HashMap;

    #[test]
    fn empty_string_returns_empty_map() {
        let m = parse_query_string_multi("");
        assert!(m.is_empty());
    }

    #[test]
    fn single_pair_parses_correctly() {
        let m = parse_query_string_multi("user=a");
        assert_eq!(m.len(), 1);
        assert_eq!(m.get("user"), Some(&vec!["a".to_string()]));
    }

    #[test]
    fn repeated_keys_accumulate_in_order() {
        let m = parse_query_string_multi("id=1&id=2&id=3");
        assert_eq!(m.len(), 1);
        assert_eq!(
            m.get("id"),
            Some(&vec!["1".to_string(), "2".to_string(), "3".to_string()])
        );
    }

    #[test]
    fn multiple_distinct_keys() {
        let m = parse_query_string_multi("user=a&role=admin&user=b");
        assert_eq!(m.len(), 2);
        assert_eq!(m.get("role"), Some(&vec!["admin".to_string()]));
        assert_eq!(m.get("user"), Some(&vec!["a".to_string(), "b".to_string()]));
    }

    #[test]
    fn percent_decoding_and_plus_as_space() {
        // url::form_urlencoded decodes %XX and treats '+' as space.
        let m = parse_query_string_multi("name=Andr%C3%A9s+Pirela&k%2Bey=v%2Balue");
        assert_eq!(m.get("name"), Some(&vec!["Andrés Pirela".to_string()]));
        // %2B becomes '+', not space.
        assert_eq!(m.get("k+ey"), Some(&vec!["v+alue".to_string()]));
    }

    #[test]
    fn empty_value_is_kept_as_empty_string() {
        let m = parse_query_string_multi("a=&b=");
        assert_eq!(m.get("a"), Some(&vec!["".to_string()]));
        assert_eq!(m.get("b"), Some(&vec!["".to_string()]));
    }

    #[test]
    fn bare_key_gets_empty_string_value() {
        // "flag" without '=' should parse as ("flag", "")
        let m = parse_query_string_multi("flag&x=1");
        assert_eq!(m.get("flag"), Some(&vec!["".to_string()]));
        assert_eq!(m.get("x"), Some(&vec!["1".to_string()]));
    }

    #[test]
    fn unicode_keys_and_values() {
        let m = parse_query_string_multi("名=値&emoji=%F0%9F%98%80"); // 😀
        assert_eq!(m.get("名"), Some(&vec!["値".to_string()]));
        assert_eq!(m.get("emoji"), Some(&vec!["😀".to_string()]));
    }

    #[test]
    fn preserves_insertion_order_within_value_vectors() {
        // Ensure the per-key Vec preserves the order the pairs appear in the string.
        let m = parse_query_string_multi("k=first&x=1&k=second&k=third&x=2");
        assert_eq!(
            m.get("k"),
            Some(&vec![
                "first".to_string(),
                "second".to_string(),
                "third".to_string()
            ])
        );
        assert_eq!(m.get("x"), Some(&vec!["1".to_string(), "2".to_string()]));
    }

    #[test]
    fn mixed_empty_and_nonempty_values() {
        let m = parse_query_string_multi("k=&k=1&&k=2"); // note the empty pair between &&
        assert_eq!(
            m.get("k"),
            Some(&vec!["".to_string(), "1".to_string(), "2".to_string()])
        );
    }

    #[test]
    fn plus_in_key_and_value_becomes_space() {
        let m = parse_query_string_multi("a+b=c+d&a+b=e+f");
        assert_eq!(
            m.get("a b"),
            Some(&vec!["c d".to_string(), "e f".to_string()])
        );
    }
}
