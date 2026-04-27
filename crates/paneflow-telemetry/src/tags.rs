//! Canonical-tag format invariant helper (US-013).
//!
//! Domain mapping (e.g. `InstallMethod` → `"deb"`/`"rpm"`/…) lives in the
//! consumer crate where the domain types are defined; this module owns
//! only the format contract every published tag must satisfy.
//!
//! The contract: a canonical telemetry tag is a non-empty string composed
//! exclusively of lowercase ASCII letters, ASCII digits, hyphens, and
//! dots. PostHog breakdowns are case-sensitive and do not normalize, so
//! mixing case or whitespace would silently fragment dashboards.
//!
//! Use [`is_canonical_tag_format`] in unit tests of the domain-mapping
//! functions to assert the invariant once per variant.
//!
//! See `tasks/compliance-analytics.md §5` for the committed vocabulary.

/// Returns `true` iff `s` is a non-empty string of `[a-z0-9.-]` codepoints.
///
/// This is the format contract every published telemetry tag must satisfy.
/// Domain-mapping functions in consumer crates assert it as part of their
/// unit tests, e.g.:
///
/// ```
/// use paneflow_telemetry::tags::is_canonical_tag_format;
/// assert!(is_canonical_tag_format("deb"));
/// assert!(is_canonical_tag_format("rpm-ostree"));
/// assert!(is_canonical_tag_format("tar.gz"));
/// assert!(!is_canonical_tag_format(""));
/// assert!(!is_canonical_tag_format("DEB"));
/// assert!(!is_canonical_tag_format("with space"));
/// ```
pub fn is_canonical_tag_format(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_string_is_not_canonical() {
        assert!(!is_canonical_tag_format(""));
    }

    #[test]
    fn lowercase_alpha_only_is_canonical() {
        assert!(is_canonical_tag_format("network"));
        assert!(is_canonical_tag_format("disk"));
        assert!(is_canonical_tag_format("signature"));
    }

    #[test]
    fn lowercase_with_dot_or_hyphen_is_canonical() {
        assert!(is_canonical_tag_format("tar.gz"));
        assert!(is_canonical_tag_format("rpm-ostree"));
        assert!(is_canonical_tag_format("a.b-c"));
    }

    #[test]
    fn uppercase_is_rejected() {
        assert!(!is_canonical_tag_format("DEB"));
        assert!(!is_canonical_tag_format("Deb"));
        assert!(!is_canonical_tag_format("rpm-Ostree"));
    }

    #[test]
    fn whitespace_is_rejected() {
        assert!(!is_canonical_tag_format(" deb"));
        assert!(!is_canonical_tag_format("deb "));
        assert!(!is_canonical_tag_format("with space"));
        assert!(!is_canonical_tag_format("with\ttab"));
    }

    #[test]
    fn special_characters_are_rejected() {
        assert!(!is_canonical_tag_format("deb!"));
        assert!(!is_canonical_tag_format("deb,rpm"));
        assert!(!is_canonical_tag_format("rpm/ostree"));
        assert!(!is_canonical_tag_format("deb#"));
        assert!(!is_canonical_tag_format("emoji😀"));
    }

    #[test]
    fn digits_are_canonical() {
        assert!(is_canonical_tag_format("v1"));
        assert!(is_canonical_tag_format("0"));
        assert!(is_canonical_tag_format("tar.gz2"));
    }
}
