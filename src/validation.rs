use validator::ValidateEmail;

pub fn valid_name(value: &str) -> bool {
    let len = value.chars().count();
    (1..=100).contains(&len)
}

pub fn valid_email(value: &str) -> bool {
    value.validate_email()
}

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
    use super::{valid_name, valid_phone_number};

    #[test]
    fn name_validation_matches_length_rules() {
        assert!(valid_name("a"));
        assert!(valid_name(&"a".repeat(100)));
        assert!(!valid_name(""));
        assert!(!valid_name(&"a".repeat(101)));
    }

    #[test]
    fn phone_validation_matches_e164_rules() {
        assert!(valid_phone_number("+358401234567"));
        assert!(!valid_phone_number("358401234567"));
        assert!(!valid_phone_number("+058401234567"));
        assert!(!valid_phone_number("+12"));
    }
}
