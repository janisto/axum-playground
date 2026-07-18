use validator::ValidateEmail;

#[must_use]
pub fn normalize_name(value: &str) -> Option<String> {
    if value.chars().any(char::is_control) {
        return None;
    }
    let value = value.trim();
    let len = value.chars().count();
    (1..=100).contains(&len).then(|| value.to_owned())
}

#[must_use]
pub fn valid_email(value: &str) -> bool {
    value.validate_email()
}

#[must_use]
pub fn valid_phone_number(value: &str) -> bool {
    let Some(rest) = value.strip_prefix('+') else {
        return false;
    };

    let mut digits = rest.chars();
    let Some(first) = digits.next() else {
        return false;
    };
    if !matches!(first, '1'..='9') {
        return false;
    }

    let mut total_digits = 1;
    for digit in digits {
        if !digit.is_ascii_digit() {
            return false;
        }
        total_digits += 1;
    }

    (7..=15).contains(&total_digits)
}

#[cfg(test)]
mod tests {
    use super::{normalize_name, valid_email, valid_phone_number};

    #[test]
    fn names_are_trimmed_and_reject_blank_control_or_oversized_values() {
        assert_eq!(normalize_name("  Jane Doe  "), Some("Jane Doe".to_owned()));
        assert_eq!(normalize_name(&"a".repeat(100)), Some("a".repeat(100)));
        assert_eq!(normalize_name(""), None);
        assert_eq!(normalize_name(" \t\n "), None);
        assert_eq!(normalize_name("Jane\nDoe"), None);
        assert_eq!(normalize_name("Jane\t"), None);
        assert_eq!(normalize_name(&"a".repeat(101)), None);
    }

    #[test]
    fn phone_validation_matches_e164_rules() {
        assert!(valid_phone_number("+358401234567"));
        assert!(!valid_phone_number("358401234567"));
        assert!(!valid_phone_number("+058401234567"));
        assert!(!valid_phone_number("+12"));
    }

    #[test]
    fn email_validation_rejects_non_addresses() {
        assert!(valid_email("user@example.com"));
        assert!(!valid_email("not-an-email"));
        assert!(!valid_email("@example.com"));
    }
}
