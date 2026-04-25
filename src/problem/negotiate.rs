#[derive(Clone, Debug, PartialEq)]
pub struct MediaRange {
    pub typ: String,
    pub subtype: String,
    pub q: f64,
}

pub fn parse_accept(header: &str) -> Vec<MediaRange> {
    if header.is_empty() {
        return Vec::new();
    }

    let mut ranges = Vec::new();

    for part in header.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        let mut media_range = MediaRange {
            typ: String::new(),
            subtype: "*".to_string(),
            q: 1.0,
        };

        let mut media_type = part;
        if let Some((before, after)) = part.split_once(';') {
            media_type = before.trim();
            for param in after.split(';') {
                let param = param.trim();
                if let Some(value) = param
                    .strip_prefix("q=")
                    .or_else(|| param.strip_prefix("Q="))
                    && let Ok(parsed) = value.parse::<f64>()
                    && (0.0..=1.0).contains(&parsed)
                {
                    media_range.q = parsed;
                }
            }
        }

        if let Some((typ, subtype)) = media_type.split_once('/') {
            media_range.typ = typ.trim().to_ascii_lowercase();
            media_range.subtype = subtype.trim().to_ascii_lowercase();
        } else {
            media_range.typ = media_type.trim().to_ascii_lowercase();
        }

        ranges.push(media_range);
    }

    ranges
}

pub fn select_format(header: &str) -> bool {
    let ranges = parse_accept(header);
    if ranges.is_empty() {
        return false;
    }

    let mut cbor_q = -1.0;
    let mut json_q = -1.0;
    let mut cbor_specificity = 0;
    let mut json_specificity = 0;

    for range in ranges {
        if range.q == 0.0 {
            continue;
        }

        let mut specificity = 0;
        let mut matches_cbor = false;
        let mut matches_json = false;

        match (range.typ.as_str(), range.subtype.as_str()) {
            ("application", "problem+cbor") => {
                matches_cbor = true;
                specificity = 4;
            }
            ("application", "problem+json") => {
                matches_json = true;
                specificity = 4;
            }
            ("application", "cbor") => {
                matches_cbor = true;
                specificity = 3;
            }
            ("application", "json") => {
                matches_json = true;
                specificity = 3;
            }
            ("application", "*") => {
                matches_cbor = true;
                matches_json = true;
                specificity = 2;
            }
            ("*", "*") => {
                matches_cbor = true;
                matches_json = true;
                specificity = 1;
            }
            ("application", subtype) if subtype.ends_with("+cbor") => {
                matches_cbor = true;
                specificity = 3;
            }
            ("application", subtype) if subtype.ends_with("+json") => {
                matches_json = true;
                specificity = 3;
            }
            _ => {}
        }

        if matches_cbor
            && (specificity > cbor_specificity
                || (specificity == cbor_specificity && range.q > cbor_q))
        {
            cbor_q = range.q;
            cbor_specificity = specificity;
        }

        if matches_json
            && (specificity > json_specificity
                || (specificity == json_specificity && range.q > json_q))
        {
            json_q = range.q;
            json_specificity = specificity;
        }
    }

    if cbor_q <= 0.0 && json_q <= 0.0 {
        return false;
    }

    if cbor_q > json_q {
        return true;
    }
    if json_q > cbor_q {
        return false;
    }

    if cbor_specificity > json_specificity {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::{parse_accept, select_format};

    #[test]
    fn parse_accept_ignores_empty_parts_and_invalid_q_values() {
        let ranges = parse_accept("application/json, , text/html;q=invalid");

        assert_eq!(ranges.len(), 2);
        assert_eq!(ranges[0].typ, "application");
        assert_eq!(ranges[0].subtype, "json");
        assert_eq!(ranges[0].q, 1.0);
        assert_eq!(ranges[1].typ, "text");
        assert_eq!(ranges[1].subtype, "html");
        assert_eq!(ranges[1].q, 1.0);
    }

    #[test]
    fn parse_accept_defaults_type_without_subtype_to_wildcard_subtype() {
        let ranges = parse_accept("text");

        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].typ, "text");
        assert_eq!(ranges[0].subtype, "*");
    }

    #[test]
    fn select_format_defaults_to_json() {
        assert!(!select_format(""));
        assert!(!select_format("text/html"));
    }

    #[test]
    fn select_format_prefers_cbor_when_q_value_is_higher() {
        assert!(select_format(
            "application/json;q=0.9, application/cbor;q=1.0"
        ));
    }

    #[test]
    fn select_format_honors_problem_media_types() {
        assert!(select_format("application/problem+cbor"));
        assert!(!select_format("application/problem+json"));
        assert!(select_format(
            "application/problem+cbor;q=1.0, application/problem+json;q=0.5"
        ));
        assert!(!select_format(
            "application/problem+cbor;q=0.5, application/problem+json;q=1.0"
        ));
    }

    #[test]
    fn select_format_uses_specificity_as_tie_breaker() {
        assert!(select_format("application/cbor, application/problem+cbor"));
        assert!(!select_format(
            "application/cbor;q=0.8, application/problem+json;q=0.8"
        ));
        assert!(select_format(
            "application/json;q=0.8, application/problem+cbor;q=0.8"
        ));
    }

    #[test]
    fn select_format_supports_structured_suffix_wildcards() {
        assert!(select_format("application/*+cbor"));
        assert!(!select_format("application/*+json"));
    }

    #[test]
    fn select_format_respects_q_zero_and_invalid_q_defaults() {
        assert!(!select_format("application/json;q=0, application/cbor;q=0"));
        assert!(!select_format("*/*;q=0"));
        assert!(select_format("application/cbor;q=invalid"));
        assert!(!select_format(
            "application/cbor;q=0, application/json;q=1.0"
        ));
        assert!(select_format(
            "application/json;q=0, application/cbor;q=1.0"
        ));
    }
}
