#[must_use]
pub fn build_link_header(
    base_url: &str,
    query: &[(&str, &str)],
    next_cursor: Option<&str>,
    prev_cursor: Option<&str>,
) -> String {
    let mut links = Vec::new();

    if let Some(next_cursor) = next_cursor.filter(|cursor| !cursor.is_empty()) {
        let query = replace_cursor(query, next_cursor);
        links.push(format!("<{base_url}?{query}>; rel=\"next\""));
    }

    if let Some(prev_cursor) = prev_cursor.filter(|cursor| !cursor.is_empty()) {
        let query = replace_cursor(query, prev_cursor);
        links.push(format!("<{base_url}?{query}>; rel=\"prev\""));
    }

    links.join(", ")
}

fn replace_cursor(query: &[(&str, &str)], cursor: &str) -> String {
    let mut pairs = query
        .iter()
        .filter(|(key, _)| *key != "cursor")
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect::<Vec<_>>();

    pairs.push(("cursor".to_owned(), cursor.to_owned()));

    serialize_query(&pairs)
}

fn serialize_query(pairs: &[(String, String)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| {
            format!(
                "{}={}",
                encode_query_component(key),
                encode_query_component(value)
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn encode_query_component(value: &str) -> String {
    let mut encoded = String::new();

    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }

    encoded
}

#[cfg(test)]
mod tests {
    use super::build_link_header;
    use crate::pagination::cursor::Cursor;

    #[test]
    fn link_header_includes_next_and_prev_relations() {
        let link = build_link_header("/items", &[("limit", "10")], Some("next"), Some("prev"));

        assert!(link.contains("</items?limit=10&cursor=next>; rel=\"next\""));
        assert!(link.contains("</items?limit=10&cursor=prev>; rel=\"prev\""));
    }

    #[test]
    fn link_header_replaces_existing_cursor_parameter() {
        let link = build_link_header(
            "/items",
            &[("cursor", "old-cursor"), ("limit", "10")],
            Some("new-cursor"),
            None,
        );

        assert!(!link.contains("old-cursor"));
        assert!(link.contains("cursor=new-cursor"));
    }

    #[test]
    fn link_header_handles_empty_base_url_for_relative_links() {
        let link = build_link_header("", &[], Some("next"), None);
        assert!(link.contains("<?cursor=next>; rel=\"next\""));
    }

    #[test]
    fn link_header_keeps_cursor_url_safe() {
        let cursor = Cursor::new("item", "abc/def+ghi=jkl").encode();
        let link = build_link_header("/items", &[], Some(&cursor), None);

        assert!(link.contains("cursor="));
        assert!(!link.contains('+'));
    }

    #[test]
    fn link_header_form_encodes_spaces_in_preserved_filters() {
        let link = build_link_header("/items", &[("category", "power tools")], Some("next"), None);

        assert_eq!(
            link,
            "</items?category=power+tools&cursor=next>; rel=\"next\""
        );
    }
}
