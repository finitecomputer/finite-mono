//! Site redirect targets: where an owner may send every request for their
//! Site once it has moved (ADR 0030). A target is a bare `https` origin, so
//! serving appends the request's own path and query, and nothing a visitor
//! sends can change the host it is sent to.

use crate::ProtoError;
use crate::limits::MAX_REDIRECT_TARGET_BYTES;

const SCHEME: &str = "https://";
/// RFC 1035: a DNS label is 1 to 63 octets.
const MAX_LABEL_LENGTH: usize = 63;
/// RFC 1035: a host name is at most 253 octets in text form.
const MAX_HOST_LENGTH: usize = 253;
/// A host has at most this many labels: 253 octets of one-letter labels.
const MAX_LABELS: usize = 127;

/// Validate a redirect target and return its canonical form,
/// `https://{host}` with the host lowercased and no trailing slash.
///
/// Accepted: `https://` and a DNS host name of two or more labels, with an
/// optional trailing `/`. Refused: any other scheme, user info, a port, a
/// path, a query, a fragment, an IP address, and anything longer than
/// `MAX_REDIRECT_TARGET_BYTES`.
pub fn validate_redirect_target(input: &str) -> Result<String, ProtoError> {
    if input.len() > MAX_REDIRECT_TARGET_BYTES {
        return Err(ProtoError::InvalidRedirectTarget("too long"));
    }
    let rest = strip_scheme(input).ok_or(ProtoError::InvalidRedirectTarget(
        "must start with https://",
    ))?;
    let host = rest.strip_suffix('/').unwrap_or(rest);
    if host.contains(['@', ':', '/', '?', '#', '\\']) {
        return Err(ProtoError::InvalidRedirectTarget(
            "must be an origin: no user info, port, path, query or fragment",
        ));
    }
    let host = host.to_ascii_lowercase();
    validate_host(&host)?;
    let canonical = format!("{SCHEME}{host}");
    assert!(canonical.len() <= MAX_REDIRECT_TARGET_BYTES);
    assert!(!canonical.ends_with('/'));
    Ok(canonical)
}

/// The host of a canonical target, as `validate_redirect_target` returns it.
pub fn redirect_target_host(canonical: &str) -> &str {
    let host = canonical
        .strip_prefix(SCHEME)
        .expect("a canonical redirect target starts with https://");
    assert!(!host.is_empty());
    host
}

/// True when `host` is `domain` itself or a name under it. Both are
/// compared as lowercase DNS names.
pub fn host_is_within(host: &str, domain: &str) -> bool {
    assert!(!domain.is_empty());
    let host = host.to_ascii_lowercase();
    let domain = domain.to_ascii_lowercase();
    host == domain || host.ends_with(&format!(".{domain}"))
}

fn strip_scheme(input: &str) -> Option<&str> {
    let head = input.get(..SCHEME.len())?;
    if head.eq_ignore_ascii_case(SCHEME) {
        Some(&input[SCHEME.len()..])
    } else {
        None
    }
}

fn validate_host(host: &str) -> Result<(), ProtoError> {
    if host.is_empty() || host.len() > MAX_HOST_LENGTH {
        return Err(ProtoError::InvalidRedirectTarget(
            "host must be 1 to 253 characters",
        ));
    }
    let mut labels = 0usize;
    let mut last = "";
    for label in host.split('.').take(MAX_LABELS + 1) {
        labels += 1;
        if labels > MAX_LABELS {
            return Err(ProtoError::InvalidRedirectTarget(
                "host has too many labels",
            ));
        }
        validate_label(label)?;
        last = label;
    }
    if labels < 2 {
        return Err(ProtoError::InvalidRedirectTarget(
            "host must be a domain name with at least two labels",
        ));
    }
    // A browser reads a host whose last label is a number as an IPv4
    // address (WHATWG URL "ends in a number"): `1.2.3.4`, `1.2.3.0x4`.
    if ends_in_a_number(last) {
        return Err(ProtoError::InvalidRedirectTarget(
            "host must be a domain name, not an IP address",
        ));
    }
    Ok(())
}

fn ends_in_a_number(label: &str) -> bool {
    label.bytes().all(|b| b.is_ascii_digit())
        || label
            .strip_prefix("0x")
            .is_some_and(|hex| hex.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn validate_label(label: &str) -> Result<(), ProtoError> {
    if label.is_empty() || label.len() > MAX_LABEL_LENGTH {
        return Err(ProtoError::InvalidRedirectTarget(
            "each host label must be 1 to 63 characters",
        ));
    }
    if !label
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(ProtoError::InvalidRedirectTarget(
            "host may contain only letters, digits, hyphens and dots",
        ));
    }
    if label.starts_with('-') || label.ends_with('-') {
        return Err(ProtoError::InvalidRedirectTarget(
            "a host label may not start or end with a hyphen",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(input: &str) -> bool {
        matches!(
            validate_redirect_target(input),
            Err(ProtoError::InvalidRedirectTarget(_))
        )
    }

    #[test]
    fn accepts_an_https_origin_in_canonical_form() {
        assert_eq!(
            validate_redirect_target("https://garden.finite.place").unwrap(),
            "https://garden.finite.place"
        );
        assert_eq!(
            validate_redirect_target("HTTPS://Garden.Finite.Place/").unwrap(),
            "https://garden.finite.place"
        );
        assert_eq!(
            validate_redirect_target("https://xn--bcher-kva.example").unwrap(),
            "https://xn--bcher-kva.example"
        );
        // A label that merely starts like a number is a name.
        assert!(validate_redirect_target("https://0xcafe-shop.example").is_ok());
        assert!(validate_redirect_target("https://1password.example").is_ok());
    }

    #[test]
    fn replaying_the_canonical_form_is_a_fixed_point() {
        let once = validate_redirect_target("https://Moved.Example.COM/").unwrap();
        assert_eq!(validate_redirect_target(&once).unwrap(), once);
    }

    #[test]
    fn refuses_other_schemes_and_anything_past_the_origin() {
        for input in [
            "",
            "http://garden.finite.place",
            "//garden.finite.place",
            "garden.finite.place",
            "javascript:alert(1)",
            "https://",
            "https:///",
            "https://garden.finite.place/blog",
            "https://garden.finite.place//",
            "https://garden.finite.place?x=1",
            "https://garden.finite.place#top",
            "https://garden.finite.place:8443",
            "https://user@garden.finite.place",
            "https://user:pw@garden.finite.place",
            "https://garden.finite.place\\@evil.example",
            "https://garden.finite.place /",
        ] {
            assert!(refused(input), "{input:?} must be refused");
        }
    }

    #[test]
    fn refuses_hosts_that_are_not_domain_names() {
        for input in [
            "https://localhost",
            "https://127.0.0.1",
            "https://10.0.0.1/",
            "https://1.2.3.0x4",
            "https://example.0x",
            "https://example.0xff",
            "https://0x7f.1",
            "https://[::1]",
            "https://-garden.finite.place",
            "https://garden-.finite.place",
            "https://garden..finite.place",
            "https://.finite.place",
            "https://garden.finite.place.",
            "https://gärden.finite.place",
            "https://garden_site.finite.place",
            "https://garden.finite.place\n",
        ] {
            assert!(refused(input), "{input:?} must be refused");
        }
    }

    #[test]
    fn enforces_label_host_and_total_length_bounds() {
        let label63 = "a".repeat(63);
        assert!(validate_redirect_target(&format!("https://{label63}.example")).is_ok());
        assert!(refused(&format!("https://{}.example", "a".repeat(64))));

        let host253 = format!("{}.{}.{}.{}", label63, label63, label63, "b".repeat(61));
        assert_eq!(host253.len(), 253);
        assert!(validate_redirect_target(&format!("https://{host253}")).is_ok());
        let host254 = format!("{host253}b");
        assert!(refused(&format!("https://{host254}")));

        let many_labels = vec!["a"; 200].join(".");
        assert!(refused(&format!("https://{many_labels}")));
        assert!(refused(&format!(
            "https://{}",
            "a".repeat(MAX_REDIRECT_TARGET_BYTES)
        )));
    }

    #[test]
    fn the_request_names_its_target_explicitly() {
        use crate::dto::SiteRedirectRequest;
        let set: SiteRedirectRequest =
            serde_json::from_str(r#"{"to":"https://garden.finite.place"}"#).unwrap();
        assert_eq!(set.to.as_deref(), Some("https://garden.finite.place"));
        let clear: SiteRedirectRequest = serde_json::from_str(r#"{"to":null}"#).unwrap();
        assert_eq!(clear.to, None);
        // An empty body never clears a redirect by accident, and a typo is
        // refused rather than ignored.
        assert!(serde_json::from_str::<SiteRedirectRequest>("{}").is_err());
        assert!(
            serde_json::from_str::<SiteRedirectRequest>(
                r#"{"to":null,"target":"https://x.example"}"#
            )
            .is_err()
        );
    }

    #[test]
    fn host_containment_is_by_whole_labels() {
        assert!(host_is_within("finite.site", "finite.site"));
        assert!(host_is_within("garden.finite.site", "finite.site"));
        assert!(host_is_within("Garden.Finite.Site", "finite.site"));
        assert!(!host_is_within("notfinite.site", "finite.site"));
        assert!(!host_is_within("finite.site.example", "finite.site"));
        assert_eq!(
            redirect_target_host("https://garden.finite.place"),
            "garden.finite.place"
        );
    }
}
