use std::collections::BTreeSet;

use axum::http::{HeaderMap, header};

pub const CBOR_MEDIA_TYPE: &str = "application/cbor";
pub const JSON_MEDIA_TYPE: &str = "application/json";
pub const JSON_UTF8_MEDIA_TYPE: &str = "application/json;charset=utf-8";
pub const PROBLEM_JSON_MEDIA_TYPE: &str = "application/problem+json";
pub const PROBLEM_JSON_UTF8_MEDIA_TYPE: &str = "application/problem+json;charset=utf-8";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Representation {
    Json,
    JsonUtf8,
    Cbor,
}

impl Representation {
    #[must_use]
    pub const fn success_content_type(self) -> &'static str {
        match self {
            Self::Json => JSON_MEDIA_TYPE,
            Self::JsonUtf8 => JSON_UTF8_MEDIA_TYPE,
            Self::Cbor => CBOR_MEDIA_TYPE,
        }
    }

    #[must_use]
    pub const fn problem_content_type(self) -> &'static str {
        match self {
            Self::Json => PROBLEM_JSON_MEDIA_TYPE,
            Self::JsonUtf8 => PROBLEM_JSON_UTF8_MEDIA_TYPE,
            Self::Cbor => CBOR_MEDIA_TYPE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Match {
    media_parameters: u8,
    quality: f32,
    specificity: u8,
}

#[derive(Debug)]
struct ParsedRange {
    media_type: String,
    media_parameters: Vec<(String, String)>,
    quality: f32,
}

#[must_use]
pub fn negotiate_api_representation(
    headers: &HeaderMap,
    allow_cbor: bool,
) -> Option<Representation> {
    negotiate_success(headers, allow_cbor, JSON_MEDIA_TYPE)
}

#[must_use]
pub fn negotiate_json_representation(headers: &HeaderMap) -> Option<Representation> {
    negotiate_success(headers, false, JSON_MEDIA_TYPE)
}

#[must_use]
pub fn negotiate_problem_representation(headers: &HeaderMap) -> Representation {
    let Some(ranges) = accept_ranges(headers) else {
        return Representation::Json;
    };
    choose_representation(&ranges, true, PROBLEM_JSON_MEDIA_TYPE).unwrap_or(Representation::Json)
}

fn negotiate_success(
    headers: &HeaderMap,
    allow_cbor: bool,
    json_media_type: &str,
) -> Option<Representation> {
    let Some(ranges) = accept_ranges(headers) else {
        return Some(Representation::Json);
    };
    choose_representation(&ranges, allow_cbor, json_media_type)
}

fn choose_representation(
    ranges: &[ParsedRange],
    allow_cbor: bool,
    json_media_type: &str,
) -> Option<Representation> {
    let candidates = [
        (Representation::Json, json_media_type, false),
        (Representation::JsonUtf8, json_media_type, true),
        (Representation::Cbor, CBOR_MEDIA_TYPE, false),
    ];

    let mut selected = None;
    let mut selected_quality = 0.0;
    for (representation, media_type, charset) in candidates {
        if representation == Representation::Cbor && !allow_cbor {
            continue;
        }
        let quality = effective_quality(ranges, media_type, charset).unwrap_or(0.0);
        if quality > selected_quality {
            selected = Some(representation);
            selected_quality = quality;
        }
    }
    selected
}

fn effective_quality(
    ranges: &[ParsedRange],
    media_type: &str,
    with_utf8_charset: bool,
) -> Option<f32> {
    let mut best: Option<Match> = None;
    for range in ranges {
        let Some(specificity) = range_specificity(&range.media_type, media_type) else {
            continue;
        };
        if !range_parameters_match(&range.media_parameters, with_utf8_charset) {
            continue;
        }
        let current = Match {
            media_parameters: range.media_parameters.len() as u8,
            quality: range.quality,
            specificity,
        };
        best = Some(best.map_or(current, |mut best| {
            let current_precedence = (current.specificity, current.media_parameters);
            let best_precedence = (best.specificity, best.media_parameters);
            if current_precedence > best_precedence {
                current
            } else {
                if current_precedence == best_precedence {
                    best.quality = best.quality.max(current.quality);
                }
                best
            }
        }));
    }
    best.map(|value| value.quality)
}

fn range_parameters_match(parameters: &[(String, String)], with_utf8_charset: bool) -> bool {
    match parameters {
        [] => true,
        [(name, value)] => {
            with_utf8_charset && name == "charset" && value.eq_ignore_ascii_case("utf-8")
        }
        _ => false,
    }
}

fn range_specificity(range: &str, target: &str) -> Option<u8> {
    if range == target {
        return Some(2);
    }
    if range == "*/*" {
        return Some(0);
    }
    let (range_type, range_subtype) = range.split_once('/')?;
    let (target_type, _) = target.split_once('/')?;
    (range_type == target_type && range_subtype == "*").then_some(1)
}

fn accept_ranges(headers: &HeaderMap) -> Option<Vec<ParsedRange>> {
    let values = headers.get_all(header::ACCEPT);
    values.iter().next()?;

    Some(
        values
            .iter()
            .filter_map(|value| value.to_str().ok())
            .flat_map(split_quoted_commas)
            .filter_map(|value| parse_range(&value))
            .collect(),
    )
}

fn parse_range(value: &str) -> Option<ParsedRange> {
    let parts = split_outside_quotes(value.trim(), ';')?;
    let media_type = parts.first()?.trim().to_ascii_lowercase();
    let (type_name, subtype) = media_type.split_once('/')?;
    if !valid_token(type_name)
        || !(valid_token(subtype) || subtype == "*")
        || (type_name == "*" && subtype != "*")
    {
        return None;
    }

    let mut quality = 1.0;
    let mut quality_seen = false;
    let mut names = BTreeSet::new();
    let mut media_parameters = Vec::new();
    for part in parts.iter().skip(1) {
        let part = part.trim();
        let (name, raw_value) = part.split_once('=')?;
        let name = name.trim().to_ascii_lowercase();
        if !valid_token(&name) || !names.insert(name.clone()) {
            return None;
        }
        if name == "q" {
            if quality_seen {
                return None;
            }
            quality = parse_quality(raw_value.trim())?;
            quality_seen = true;
        } else {
            let value = decode_parameter(raw_value.trim())?;
            if !quality_seen {
                media_parameters.push((name, value));
            }
        }
    }

    Some(ParsedRange {
        media_type,
        media_parameters,
        quality,
    })
}

fn split_quoted_commas(value: &str) -> Vec<String> {
    split_outside_quotes(value, ',').unwrap_or_else(|| vec![value.to_owned()])
}

pub(crate) fn split_outside_quotes(value: &str, delimiter: char) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (index, character) in value.char_indices() {
        if quoted {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
        } else if character == '"' {
            quoted = true;
        } else if character == delimiter {
            parts.push(value[start..index].to_owned());
            start = index + character.len_utf8();
        }
    }
    if quoted || escaped {
        return None;
    }
    parts.push(value[start..].to_owned());
    Some(parts)
}

pub(crate) fn decode_parameter(value: &str) -> Option<String> {
    if valid_token(value) {
        return Some(value.to_owned());
    }
    let inner = value.strip_prefix('"')?.strip_suffix('"')?;
    let mut decoded = String::new();
    let mut escaped = false;
    for character in inner.chars() {
        if escaped {
            if character.is_control() && character != '\t' {
                return None;
            }
            decoded.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '"' || (character.is_control() && character != '\t') {
            return None;
        } else {
            decoded.push(character);
        }
    }
    (!escaped).then_some(decoded)
}

pub(crate) fn valid_token(value: &str) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn parse_quality(value: &str) -> Option<f32> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if fraction.len() > 3 || !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    match whole {
        "0" => value.parse().ok(),
        "1" if fraction.bytes().all(|byte| byte == b'0') => value.parse().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{HeaderMap, HeaderValue, header};

    use super::{
        Representation, decode_parameter, effective_quality, negotiate_api_representation,
        negotiate_json_representation, negotiate_problem_representation, parse_quality,
        parse_range, split_outside_quotes, valid_token,
    };

    fn headers(accept: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static(accept));
        headers
    }

    #[test]
    fn success_negotiation_honors_specificity_exclusion_and_ties() {
        assert_eq!(
            negotiate_api_representation(&headers("application/cbor"), true),
            Some(Representation::Cbor)
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/json;q=0, application/*;q=1"), true),
            Some(Representation::Cbor)
        );
        assert_eq!(
            negotiate_api_representation(
                &headers("application/cbor;q=0.8, application/json;q=0.8"),
                true
            ),
            Some(Representation::Json)
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/xml"), true),
            None
        );
        assert_eq!(
            negotiate_api_representation(&headers("not valid, application/cbor"), true),
            Some(Representation::Cbor)
        );
        assert_eq!(
            negotiate_api_representation(
                &headers("application/json;q=0.4, application/cbor;q=0.8"),
                true
            ),
            Some(Representation::Cbor)
        );
        assert_eq!(
            negotiate_api_representation(
                &headers("application/json;q=0.8, application/cbor;q=0.4"),
                true
            ),
            Some(Representation::Json)
        );
    }

    #[test]
    fn json_charset_is_a_distinct_candidate() {
        assert_eq!(
            negotiate_json_representation(&headers("application/json;charset=utf-8")),
            Some(Representation::JsonUtf8)
        );
        assert_eq!(
            negotiate_json_representation(&headers("application/json;charset=latin1")),
            None
        );
    }

    #[test]
    fn problem_negotiation_does_not_treat_plain_json_as_problem_json() {
        assert_eq!(
            negotiate_problem_representation(&headers("application/cbor")),
            Representation::Cbor
        );
        assert_eq!(
            negotiate_problem_representation(&headers("application/json")),
            Representation::Json
        );
        assert_eq!(
            negotiate_problem_representation(&headers("application/problem+json;charset=utf-8")),
            Representation::JsonUtf8
        );
    }

    #[test]
    fn matching_precedence_is_specificity_then_parameters_then_quality() {
        let ranges = ["*/*;q=0.9", "application/*;q=0.8", "application/json;q=0.7"]
            .into_iter()
            .map(|value| parse_range(value).expect("range"))
            .collect::<Vec<_>>();
        assert_eq!(
            effective_quality(&ranges, "application/json", false),
            Some(0.7)
        );

        let ranges = [
            "application/json;q=0.9",
            "application/json;charset=utf-8;q=0.4",
            "application/json;charset=utf-8;q=0.6",
            "application/json;charset=utf-8;q=0.2",
        ]
        .into_iter()
        .map(|value| parse_range(value).expect("range"))
        .collect::<Vec<_>>();
        assert_eq!(
            effective_quality(&ranges, "application/json", true),
            Some(0.6)
        );
        assert_eq!(
            effective_quality(&ranges, "application/json", false),
            Some(0.9)
        );

        let ranges = ["application/json;q=0.8", "*/*;charset=utf-8;q=0.2"]
            .into_iter()
            .map(|value| parse_range(value).expect("range"))
            .collect::<Vec<_>>();
        assert_eq!(
            effective_quality(&ranges, "application/json", true),
            Some(0.8)
        );
    }

    #[test]
    fn accept_range_parser_rejects_malformed_or_ambiguous_metadata() {
        for value in [
            "*/json",
            "app lication/json",
            "application/",
            "application/json;q=0.2;q=0.3",
            "application/json;charset=utf-8;charset=UTF-8",
            "application/json;bad name=value",
            "application/json;name",
            "application/cbor;q=\"1\"",
            "application/json;name=\"unterminated",
            "application/json;name=\"trailing\\\"",
        ] {
            assert!(parse_range(value).is_none(), "{value}");
        }
        let range = parse_range("Application/JSON; Charset=\"utf\\-8\"; q=0.125; ignored=late")
            .expect("valid range");
        assert_eq!(range.media_type, "application/json");
        assert_eq!(
            range.media_parameters,
            [("charset".to_owned(), "utf-8".to_owned())]
        );
        assert_eq!(range.quality, 0.125);
    }

    #[test]
    fn quoted_splitting_and_parameter_decoding_are_strict() {
        assert_eq!(
            split_outside_quotes("one,\"two,three\",four", ','),
            Some(vec![
                "one".to_owned(),
                "\"two,three\"".to_owned(),
                "four".to_owned(),
            ])
        );
        assert_eq!(
            split_outside_quotes("\"escaped\\\"quote\";tail", ';'),
            Some(vec!["\"escaped\\\"quote\"".to_owned(), "tail".to_owned()])
        );
        assert_eq!(split_outside_quotes("\"unterminated", ','), None);
        assert_eq!(split_outside_quotes("\"trailing\\", ','), None);

        for (value, expected) in [
            ("token", Some("token")),
            ("\"quoted value\"", Some("quoted value")),
            ("\"quoted\\\"value\"", Some("quoted\"value")),
            ("\"tab\tvalue\"", Some("tab\tvalue")),
        ] {
            assert_eq!(decode_parameter(value).as_deref(), expected, "{value:?}");
        }
        for value in [
            "",
            "plain value",
            "\"unterminated",
            "unterminated\"",
            "\"trailing\\\"",
            "\"raw\"quote\"",
            "\"line\nfeed\"",
            "\"escaped\\\nfeed\"",
        ] {
            assert_eq!(decode_parameter(value), None, "{value:?}");
        }
    }

    #[test]
    fn token_and_quality_grammars_cover_exact_boundaries() {
        for value in ["a", "A-Z_0", "!#$%&'*+-.^_`|~"] {
            assert!(valid_token(value), "{value}");
        }
        for value in ["", "a b", "a/b", "a,b", "é"] {
            assert!(!valid_token(value), "{value}");
        }

        for (value, expected) in [
            ("0", Some(0.0)),
            ("0.", Some(0.0)),
            ("0.001", Some(0.001)),
            ("0.999", Some(0.999)),
            ("1", Some(1.0)),
            ("1.000", Some(1.0)),
        ] {
            assert_eq!(parse_quality(value), expected, "{value}");
        }
        for value in [
            "", ".5", "00", "0.0000", "0.a", "1.001", "1.0000", "2", "-1",
        ] {
            assert_eq!(parse_quality(value), None, "{value}");
        }
    }
}
