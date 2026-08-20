use time::{Duration, OffsetDateTime, UtcOffset, format_description::well_known::Rfc3339};

pub const SAFE_INTEGER_MAX: u64 = 9_007_199_254_740_991;
pub const MAX_TIMESTAMP: &str = "9999-12-31T23:59:59.999Z";

#[must_use]
pub fn valid_bounded_name(value: &str) -> bool {
    let length = value.chars().count();
    (1..=100).contains(&length)
        && !value.chars().any(is_control_scalar)
        && value
            .chars()
            .next()
            .is_some_and(|value| !is_whitespace_scalar(value))
        && value
            .chars()
            .next_back()
            .is_some_and(|value| !is_whitespace_scalar(value))
}

#[must_use]
pub fn normalize_contact_email(value: &str) -> Option<String> {
    let value = strip_ascii_whitespace(value);
    if !value.is_ascii()
        || value.len() > 254
        || value.bytes().filter(|byte| *byte == b'@').count() != 1
    {
        return None;
    }

    let (local, domain) = value.split_once('@')?;
    if local.is_empty()
        || local.len() > 64
        || local.starts_with('.')
        || local.ends_with('.')
        || local.contains("..")
        || !local.bytes().all(valid_email_local_byte)
    {
        return None;
    }

    let labels = domain.split('.').collect::<Vec<_>>();
    if labels.len() < 2
        || labels.iter().any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
    {
        return None;
    }

    Some(format!("{local}@{}", domain.to_ascii_lowercase()))
}

#[must_use]
pub fn normalize_phone_number(value: &str) -> Option<String> {
    let value = strip_ascii_whitespace(value);
    let rest = value.strip_prefix('+')?;
    let mut digits = rest.bytes();
    let first = digits.next()?;
    if !matches!(first, b'1'..=b'9') || !digits.all(|digit| digit.is_ascii_digit()) {
        return None;
    }
    (7..=15).contains(&rest.len()).then(|| value.to_owned())
}

#[must_use]
pub fn valid_opaque_id(value: &str) -> bool {
    let length = value.chars().count();
    (1..=128).contains(&length)
}

#[must_use]
pub fn canonical_timestamp(value: OffsetDateTime) -> Option<String> {
    let value = value.to_offset(UtcOffset::UTC);
    let year = value.year();
    if !(0..=9999).contains(&year) || !value.nanosecond().is_multiple_of(1_000_000) {
        return None;
    }

    Some(format!(
        "{year:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        u8::from(value.month()),
        value.day(),
        value.hour(),
        value.minute(),
        value.second(),
        value.millisecond(),
    ))
}

#[must_use]
pub fn canonical_clock_timestamp(value: OffsetDateTime) -> Option<String> {
    value
        .replace_nanosecond(value.nanosecond() / 1_000_000 * 1_000_000)
        .ok()
        .and_then(canonical_timestamp)
}

#[must_use]
pub fn normalize_timestamp(value: &str) -> Option<String> {
    let parsed = OffsetDateTime::parse(value, &Rfc3339).ok()?;
    canonical_timestamp(parsed)
}

#[must_use]
pub fn next_timestamp(previous: &str, now: OffsetDateTime) -> Option<String> {
    let now = canonical_clock_timestamp(now)?;
    if now.as_str() > previous {
        return Some(now);
    }
    if previous == MAX_TIMESTAMP {
        return None;
    }

    let previous = OffsetDateTime::parse(previous, &Rfc3339).ok()?;
    canonical_timestamp(previous.checked_add(Duration::milliseconds(1))?)
}

fn strip_ascii_whitespace(value: &str) -> &str {
    value.trim_matches(|character| matches!(character, '\u{0009}'..='\u{000d}' | '\u{0020}'))
}

fn valid_email_local_byte(byte: u8) -> bool {
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
                | b'/'
                | b'='
                | b'?'
                | b'^'
                | b'_'
                | b'`'
                | b'{'
                | b'|'
                | b'}'
                | b'~'
                | b'.'
        )
}

fn is_control_scalar(value: char) -> bool {
    matches!(value, '\u{0000}'..='\u{001f}' | '\u{007f}'..='\u{009f}')
}

fn is_whitespace_scalar(value: char) -> bool {
    matches!(
        value,
        '\u{0009}'..='\u{000d}'
            | '\u{0020}'
            | '\u{0085}'
            | '\u{00a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::{
        MAX_TIMESTAMP, canonical_clock_timestamp, next_timestamp, normalize_contact_email,
        normalize_phone_number, normalize_timestamp, valid_bounded_name,
    };

    #[test]
    fn names_preserve_valid_input_and_reject_exact_boundary_classes() {
        assert!(valid_bounded_name("María\u{202f}José"));
        assert!(valid_bounded_name(&"a".repeat(100)));
        for value in ["", " Ada", "Ada\u{00a0}", "Ada\u{0085}", "Ada\nLovelace"] {
            assert!(
                !valid_bounded_name(value),
                "unexpectedly accepted {value:?}"
            );
        }
        assert!(!valid_bounded_name(&"a".repeat(101)));
    }

    #[test]
    fn contact_email_normalization_preserves_local_case_only() {
        assert_eq!(
            normalize_contact_email(" \tAda.Lovelace@EXAMPLE.COM\r\n"),
            Some("Ada.Lovelace@example.com".to_owned())
        );
        for value in [
            "a@example",
            ".a@example.com",
            "a.@example.com",
            "a..b@example.com",
            "a@-example.com",
            "a@example-.com",
            "a@exam_ple.com",
            "a@@example.com",
            "maria@exämple.com",
            "a,b@example.com",
        ] {
            assert_eq!(normalize_contact_email(value), None, "accepted {value}");
        }

        let local_64 = "a".repeat(64);
        let local_65 = "a".repeat(65);
        assert!(normalize_contact_email(&format!("{local_64}@example.com")).is_some());
        assert_eq!(
            normalize_contact_email(&format!("{local_65}@example.com")),
            None
        );

        let total_254 = format!(
            "{local_64}@{}.{}.{}",
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61)
        );
        assert_eq!(total_254.len(), 254);
        assert!(normalize_contact_email(&total_254).is_some());
        let total_255 = format!("{total_254}e");
        assert_eq!(total_255.len(), 255);
        assert_eq!(normalize_contact_email(&total_255), None);

        assert!(normalize_contact_email(&format!("a@{}.com", "b".repeat(63))).is_some());
        assert_eq!(
            normalize_contact_email(&format!("a@{}.com", "b".repeat(64))),
            None
        );
    }

    #[test]
    fn opaque_ids_have_exact_scalar_boundaries() {
        use super::valid_opaque_id;

        assert!(!valid_opaque_id(""));
        assert!(valid_opaque_id("a"));
        assert!(valid_opaque_id(&"é".repeat(128)));
        assert!(!valid_opaque_id(&"é".repeat(129)));
    }

    #[test]
    fn phone_normalization_uses_only_ascii_surrounding_whitespace() {
        assert_eq!(
            normalize_phone_number("\t +358401234567 \r"),
            Some("+358401234567".to_owned())
        );
        assert_eq!(normalize_phone_number("\u{00a0}+358401234567"), None);
        assert_eq!(normalize_phone_number("+01234567"), None);
    }

    #[test]
    fn timestamps_are_canonical_and_monotonic() {
        assert_eq!(
            normalize_timestamp("2026-01-01T02:00:00+02:00"),
            Some("2026-01-01T00:00:00.000Z".to_owned())
        );
        assert_eq!(
            canonical_clock_timestamp(datetime!(2026-01-01 0:00:00.999_999_999 UTC)),
            Some("2026-01-01T00:00:00.999Z".to_owned())
        );
        assert_eq!(
            next_timestamp("2026-01-01T00:00:00.999Z", datetime!(2025-01-01 0:00 UTC)),
            Some("2026-01-01T00:00:01.000Z".to_owned())
        );
        assert_eq!(
            next_timestamp(
                "2026-01-01T00:00:00.999Z",
                datetime!(2026-01-01 0:00:00.999 UTC)
            ),
            Some("2026-01-01T00:00:01.000Z".to_owned())
        );
        assert_eq!(
            next_timestamp(MAX_TIMESTAMP, datetime!(2025-01-01 0:00 UTC)),
            None
        );
        assert_eq!(normalize_timestamp("2016-12-31T23:59:60Z"), None);
        assert_eq!(normalize_timestamp("2026-01-01T00:00:00.000001Z"), None);
    }
}
