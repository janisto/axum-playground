use axum::http::{HeaderMap, header};

pub const CBOR_MEDIA_TYPE: &str = "application/cbor";
pub const JSON_MEDIA_TYPE: &str = "application/json";
pub const PROBLEM_JSON_MEDIA_TYPE: &str = "application/problem+json";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Representation {
    Json,
    Cbor,
}

#[derive(Clone, Copy, Debug)]
struct MediaRangeMatch {
    media_parameter_count: usize,
    quality: f32,
    specificity: u8,
}

#[must_use]
pub fn negotiate_api_representation(
    headers: &HeaderMap,
    allow_cbor: bool,
) -> Option<Representation> {
    let accept = combined_accept_header(headers);
    if accept.is_empty() {
        return Some(Representation::Json);
    }

    let json_quality = media_type_quality(&accept, JSON_MEDIA_TYPE, false).unwrap_or(0.0);
    let cbor_quality = if allow_cbor {
        media_type_quality(&accept, CBOR_MEDIA_TYPE, true).unwrap_or(0.0)
    } else {
        0.0
    };

    if json_quality <= 0.0 && cbor_quality <= 0.0 {
        None
    } else if cbor_quality > json_quality {
        Some(Representation::Cbor)
    } else {
        Some(Representation::Json)
    }
}

#[must_use]
pub fn negotiate_problem_representation(headers: &HeaderMap) -> Representation {
    let accept = combined_accept_header(headers);
    let explicit_problem_json_quality = media_type_quality(&accept, PROBLEM_JSON_MEDIA_TYPE, true);
    let json_quality = explicit_problem_json_quality.unwrap_or_else(|| {
        media_type_quality(&accept, PROBLEM_JSON_MEDIA_TYPE, false)
            .unwrap_or(0.0)
            .max(media_type_quality(&accept, JSON_MEDIA_TYPE, false).unwrap_or(0.0))
    });
    let cbor_quality = media_type_quality(&accept, CBOR_MEDIA_TYPE, true).unwrap_or(0.0);

    if cbor_quality > json_quality {
        Representation::Cbor
    } else {
        Representation::Json
    }
}

fn combined_accept_header(headers: &HeaderMap) -> String {
    headers
        .get_all(header::ACCEPT)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .collect::<Vec<_>>()
        .join(",")
}

fn media_type_quality(header: &str, media_type: &str, explicit_only: bool) -> Option<f32> {
    if header.is_empty() {
        return None;
    }

    let target = normalize_media_type(media_type);
    let mut best_specificity = None;
    let mut best_parameter_count = 0;
    let mut best_quality = 0.0;

    for item in header.split(',') {
        let Some(current) = match_media_range(item, &target, explicit_only) else {
            continue;
        };

        let is_more_specific = best_specificity.is_none_or(|specificity| {
            current.specificity > specificity
                || (current.specificity == specificity
                    && current.media_parameter_count > best_parameter_count)
        });
        let is_equally_specific = best_specificity.is_some_and(|specificity| {
            current.specificity == specificity
                && current.media_parameter_count == best_parameter_count
        });

        if is_more_specific {
            best_specificity = Some(current.specificity);
            best_parameter_count = current.media_parameter_count;
            best_quality = current.quality;
        } else if is_equally_specific {
            best_quality = best_quality.max(current.quality);
        }
    }

    best_specificity.map(|_| best_quality)
}

fn match_media_range(raw_item: &str, target: &str, explicit_only: bool) -> Option<MediaRangeMatch> {
    let item = raw_item.trim();
    if item.is_empty() {
        return None;
    }

    let mut parts = item.split(';');
    let range = normalize_media_type(parts.next()?);
    let specificity = range_specificity(&range, target)?;
    if explicit_only && specificity < 2 {
        return None;
    }

    let mut quality = 1.0;
    let mut quality_seen = false;
    let mut charset_seen = false;
    let mut media_parameter_count = 0;

    for raw_parameter in parts {
        let parameter = raw_parameter.trim();
        if parameter.is_empty() {
            continue;
        }

        let (name, value) = parameter.split_once('=')?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();

        if name == "q" {
            if quality_seen {
                return None;
            }
            quality = parse_quality(value)?;
            quality_seen = true;
        } else if name == "charset"
            && target == JSON_MEDIA_TYPE
            && normalize_parameter_value(value).eq_ignore_ascii_case("utf-8")
        {
            if charset_seen {
                return None;
            }
            charset_seen = true;
            media_parameter_count += 1;
        } else {
            return None;
        }
    }

    Some(MediaRangeMatch {
        media_parameter_count,
        quality,
        specificity,
    })
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
    (range_subtype == "*" && range_type == target_type).then_some(1)
}

fn normalize_media_type(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn normalize_parameter_value(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
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
        JSON_MEDIA_TYPE, Representation, media_type_quality, negotiate_api_representation,
        negotiate_problem_representation, parse_quality,
    };

    fn headers(accept: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static(accept));
        headers
    }

    #[test]
    fn api_negotiation_defaults_to_json_and_requires_explicit_cbor() {
        assert_eq!(
            negotiate_api_representation(&HeaderMap::new(), true),
            Some(Representation::Json)
        );
        assert_eq!(
            negotiate_api_representation(&headers("*/*"), true),
            Some(Representation::Json)
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/*"), true),
            Some(Representation::Json)
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/cbor"), true),
            Some(Representation::Cbor)
        );
        assert_eq!(
            negotiate_api_representation(&headers("Application/CBOR"), true),
            Some(Representation::Cbor)
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/json, application/cbor"), true),
            Some(Representation::Json)
        );
    }

    #[test]
    fn api_negotiation_honors_specific_exclusions_and_valid_quality_syntax() {
        assert_eq!(
            negotiate_api_representation(&headers("application/cbor;q=0, application/*;q=1"), true),
            Some(Representation::Json)
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/json;q=0, */*;q=1"), true),
            None
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/cbor;q=1.0000"), true),
            None
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/cbor;q=0.8"), true),
            Some(Representation::Cbor)
        );
    }

    #[test]
    fn api_negotiation_rejects_unimplemented_media_types_and_parameters() {
        assert_eq!(
            negotiate_api_representation(&headers("application/problem+json"), true),
            None
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/problem+cbor"), true),
            None
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/example+cbor"), true),
            None
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/cbor;profile=test"), true),
            None
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/json;profile=utf-8"), true),
            None
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/json;charset=iso-8859-1"), true),
            None
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/cbor;charset=utf-8"), true),
            None
        );
        assert_eq!(
            negotiate_api_representation(&headers("application/json;charset=\"UTF-8\""), true),
            Some(Representation::Json)
        );
    }

    #[test]
    fn problem_negotiation_uses_registered_cbor_or_json_fallback() {
        assert_eq!(
            negotiate_problem_representation(&headers("application/cbor")),
            Representation::Cbor
        );
        assert_eq!(
            negotiate_problem_representation(&headers("application/problem+cbor")),
            Representation::Json
        );
        assert_eq!(
            negotiate_problem_representation(&headers("application/xml")),
            Representation::Json
        );
        assert_eq!(
            negotiate_problem_representation(&headers(
                "application/problem+json;q=0, application/cbor;q=0.5"
            )),
            Representation::Cbor
        );
    }

    #[test]
    fn repeated_accept_fields_are_combined() {
        let mut headers = HeaderMap::new();
        headers.append(
            header::ACCEPT,
            HeaderValue::from_static("application/json;q=0.1"),
        );
        headers.append(
            header::ACCEPT,
            HeaderValue::from_static("application/cbor;q=1"),
        );

        assert_eq!(
            negotiate_api_representation(&headers, true),
            Some(Representation::Cbor)
        );
    }

    #[test]
    fn quality_selection_prefers_specific_ranges_and_parameters() {
        assert_eq!(
            media_type_quality(
                "*/*;q=1, application/*;q=0.8, application/json;q=0.4",
                JSON_MEDIA_TYPE,
                false,
            ),
            Some(0.4)
        );
        assert_eq!(
            media_type_quality(
                "application/json;q=0.2, application/json;q=0.8",
                JSON_MEDIA_TYPE,
                false,
            ),
            Some(0.8)
        );
        assert_eq!(
            media_type_quality(
                "application/json;q=0.8, application/json;q=0.2",
                JSON_MEDIA_TYPE,
                false,
            ),
            Some(0.8)
        );
        assert_eq!(
            media_type_quality(
                "application/json;q=0.9, application/json;charset=utf-8;q=0.1",
                JSON_MEDIA_TYPE,
                false,
            ),
            Some(0.1)
        );
        assert_eq!(
            media_type_quality(
                "application/json;q=0.3, */*;charset=utf-8;q=0.9",
                JSON_MEDIA_TYPE,
                false,
            ),
            Some(0.3)
        );
    }

    #[test]
    fn quality_parser_enforces_the_rfc_upper_bound_and_precision() {
        for (value, expected) in [
            ("0", Some(0.0)),
            ("0.123", Some(0.123)),
            ("1", Some(1.0)),
            ("1.000", Some(1.0)),
            ("1.001", None),
            ("1.1", None),
            ("0.1234", None),
            ("-0.1", None),
        ] {
            assert_eq!(
                parse_quality(value),
                expected,
                "unexpected q-value result for {value}"
            );
        }
    }
}
