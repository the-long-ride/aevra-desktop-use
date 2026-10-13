//! Hard refusals: fields that must never be read into or written through an
//! agent, and values that look like secrets.
//!
//! The helper only refuses password fields. OTP and payment fields are new
//! here. These checks are always on and not configurable.

use desktop_core::DescribeNode;

/// `password`, `otp` or `payment` when the node's name and role say it takes
/// that kind of input.
pub fn sensitive_node(node: &DescribeNode) -> Option<&'static str> {
    let role = node.role.to_lowercase();
    let text = format!("{} {}", node.name.to_lowercase(), role);

    let password_role =
        role == "password" || role == "axsecuretextfield" || role.contains("securetext");
    if password_role
        || contains_any(
            &text,
            &[
                "password",
                "passcode",
                "passwort",
                "mot de passe",
                "contraseña",
            ],
        )
        || has_word(&text, "pin")
    {
        return Some("password");
    }

    if contains_any(
        &text,
        &[
            "one-time",
            "one time",
            "verification code",
            "authenticator code",
            "security code",
        ],
    ) || has_word(&text, "otp")
        || has_word(&text, "2fa")
    {
        return Some("otp");
    }

    let card_expiry =
        (text.contains("expiry") || text.contains("expiration")) && text.contains("card");
    if contains_any(
        &text,
        &["card number", "cardnumber", "credit card", "account number"],
    ) || ["cvv", "cvc", "cvn", "iban"]
        .iter()
        .any(|w| has_word(&text, w))
        || card_expiry
    {
        return Some("payment");
    }
    None
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| haystack.contains(n))
}

/// Whole-word match: the neighbours of `word` must not be letters or digits.
fn has_word(haystack: &str, word: &str) -> bool {
    haystack.match_indices(word).any(|(start, _)| {
        let before = haystack[..start].chars().next_back();
        let after = haystack[start + word.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// Whether `value` looks like a payment card number or a credential.
///
/// This is the desktop equivalent of aevra's "values that look like secrets"
/// check on control plans.
pub fn secret_shaped(value: &str) -> bool {
    has_card_number(value) || value_tokens(value).any(token_is_secret)
}

/// Digit runs of 13 to 19 digits, optionally grouped with single spaces or
/// dashes, that pass the Luhn check.
fn has_card_number(value: &str) -> bool {
    let mut digits = String::new();
    let mut pending_separator = false;
    let flush = |digits: &mut String| {
        let found = (13..=19).contains(&digits.len()) && luhn(digits);
        digits.clear();
        found
    };
    for ch in value.chars() {
        if ch.is_ascii_digit() {
            digits.push(ch);
            pending_separator = false;
        } else if (ch == ' ' || ch == '-') && !digits.is_empty() && !pending_separator {
            pending_separator = true;
        } else {
            if flush(&mut digits) {
                return true;
            }
            pending_separator = false;
        }
    }
    flush(&mut digits)
}

fn luhn(digits: &str) -> bool {
    let mut sum = 0u32;
    for (i, ch) in digits.chars().rev().enumerate() {
        let Some(mut d) = ch.to_digit(10) else {
            return false;
        };
        if i % 2 == 1 {
            d *= 2;
            if d > 9 {
                d -= 9;
            }
        }
        sum += d;
    }
    sum.is_multiple_of(10)
}

/// Candidate tokens: runs of `[A-Za-z0-9_.-]`, so a token must start at a
/// boundary and `task-based` never counts as an `sk-` key.
fn value_tokens(value: &str) -> impl Iterator<Item = &str> {
    value
        .split(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')))
        .filter(|t| !t.is_empty())
}

fn token_is_secret(token: &str) -> bool {
    let tail_is = |prefix: &str, min: usize, ok: fn(char) -> bool| {
        token
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.len() >= min && rest.chars().all(ok))
    };
    let alnum = |c: char| c.is_ascii_alphanumeric();
    let key_char = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';

    ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"]
        .iter()
        .any(|p| tail_is(p, 20, alnum))
        || tail_is("github_pat_", 20, key_char)
        || tail_is("sk-", 20, key_char)
        || ["xoxa-", "xoxb-", "xoxp-"]
            .iter()
            .any(|p| tail_is(p, 10, key_char))
        || token.strip_prefix("AKIA").is_some_and(|rest| {
            rest.len() == 16
                && rest
                    .chars()
                    .all(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
        })
        || is_jwt(token)
}

fn is_jwt(token: &str) -> bool {
    if !token.starts_with("eyJ") {
        return false;
    }
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3 && parts[0].len() >= 5 && parts[1].len() >= 5
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(name: &str, role: &str) -> DescribeNode {
        DescribeNode {
            handle: "1:0".into(),
            role: role.into(),
            name: name.into(),
            value: None,
            enabled: true,
            focused: false,
            supported_actions: None,
            read_only: None,
            toggle_state: None,
        }
    }

    #[test]
    fn password_textbox_is_password() {
        assert_eq!(sensitive_node(&node("Password", "edit")), Some("password"));
        assert_eq!(
            sensitive_node(&node("Mot de passe", "edit")),
            Some("password")
        );
        assert_eq!(sensitive_node(&node("Enter PIN", "edit")), Some("password"));
        assert_eq!(
            sensitive_node(&node("", "AXSecureTextField")),
            Some("password")
        );
        assert_eq!(sensitive_node(&node("Field", "password")), Some("password"));
    }

    #[test]
    fn verification_code_is_otp() {
        assert_eq!(
            sensitive_node(&node("Enter 6-digit verification code", "edit")),
            Some("otp")
        );
        assert_eq!(sensitive_node(&node("OTP", "edit")), Some("otp"));
        assert_eq!(sensitive_node(&node("2FA code", "edit")), Some("otp"));
        assert_eq!(sensitive_node(&node("One-time code", "edit")), Some("otp"));
    }

    #[test]
    fn card_fields_are_payment() {
        assert_eq!(
            sensitive_node(&node("Card number", "edit")),
            Some("payment")
        );
        assert_eq!(sensitive_node(&node("CVV", "edit")), Some("payment"));
        assert_eq!(sensitive_node(&node("IBAN", "edit")), Some("payment"));
        assert_eq!(
            sensitive_node(&node("Card expiry date", "edit")),
            Some("payment")
        );
        assert_eq!(
            sensitive_node(&node("Account number", "edit")),
            Some("payment")
        );
    }

    #[test]
    fn ordinary_fields_are_not_sensitive() {
        // `pin` matches as a whole word only.
        assert_eq!(sensitive_node(&node("Spinner", "edit")), None);
        assert_eq!(sensitive_node(&node("Shopping", "edit")), None);
        assert_eq!(sensitive_node(&node("Hotpoint", "edit")), None);
        assert_eq!(sensitive_node(&node("Expiry reminder", "edit")), None);
        assert_eq!(sensitive_node(&node("Search", "edit")), None);
    }

    #[test]
    fn luhn_valid_card_number_is_secret_shaped() {
        assert!(secret_shaped("4111 1111 1111 1111"));
        assert!(secret_shaped("4111-1111-1111-1111"));
        assert!(secret_shaped("4111111111111111"));
        assert!(secret_shaped("pay with 4111 1111 1111 1111 please"));
    }

    #[test]
    fn digits_failing_luhn_or_too_short_are_not() {
        assert!(!secret_shaped("1234567890123"));
        assert!(!secret_shaped("4111 1111"));
        assert!(!secret_shaped("order 1234 5678"));
    }

    #[test]
    fn credential_prefixes_are_secret_shaped() {
        let body = "a".repeat(36);
        assert!(secret_shaped(&format!("ghp_{body}")));
        assert!(secret_shaped(&format!(
            "token {} here",
            format_args!("ghp_{body}")
        )));
        assert!(secret_shaped(&format!("github_pat_{body}")));
        assert!(secret_shaped(&format!("sk-{body}")));
        assert!(secret_shaped(&format!("xoxb-{}", "plainwords")));
        assert!(secret_shaped(&format!("AKIA{}", "WORDSWORDSWORDSX")));
        assert!(secret_shaped(&format!(
            "eyJ{w}.eyJ{w}.{w}",
            w = "plainwords"
        )));
    }

    #[test]
    fn plain_text_is_not_secret_shaped() {
        assert!(!secret_shaped("hello world"));
        assert!(!secret_shaped("hunter2"));
        assert!(!secret_shaped("a task-based workflow for the desk-kit"));
        assert!(!secret_shaped("ghp_short"));
        assert!(!secret_shaped(""));
    }
}
