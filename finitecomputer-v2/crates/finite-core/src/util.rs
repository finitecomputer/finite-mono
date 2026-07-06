use anyhow::{Result, bail};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

pub fn now_iso() -> String {
    let now = OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .unwrap_or_else(|_| OffsetDateTime::now_utc());
    now.format(&Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub fn slugify(value: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in value.trim().chars().flat_map(|ch| ch.to_lowercase()) {
        let keep = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        if keep {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

pub fn display_name_from_id(machine_id: &str) -> String {
    machine_id
        .strip_suffix("-finite")
        .unwrap_or(machine_id)
        .split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn sanitize_name(value: &str) -> String {
    value.replace(['_', '.'], "-")
}

pub fn bounded_kube_name(base: &str, suffix: &str) -> String {
    const MAX_LEN: usize = 63;
    const HASH_LEN: usize = 8;

    let base = sanitize_name(base);
    let suffix = sanitize_name(suffix).trim_matches('-').to_string();
    let suffix_segment = if suffix.is_empty() {
        String::new()
    } else {
        format!("-{suffix}")
    };
    let candidate = format!("{base}{suffix_segment}");
    if candidate.len() <= MAX_LEN {
        return candidate;
    }

    let mut hasher = Sha256::new();
    hasher.update(candidate.as_bytes());
    let hash = hex::encode(hasher.finalize());
    let hash = &hash[..HASH_LEN];
    let prefix_len = MAX_LEN
        .saturating_sub(suffix_segment.len())
        .saturating_sub(HASH_LEN + 1);
    let prefix = base
        .chars()
        .take(prefix_len)
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    let prefix = if prefix.is_empty() { "x" } else { &prefix };
    format!("{prefix}-{hash}{suffix_segment}")
}

pub fn normalize_email(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_lowercase())
}

pub fn normalize_emails(values: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut normalized = Vec::new();
    for raw in values {
        if let Some(email) = normalize_email(Some(raw))
            && seen.insert(email.clone())
        {
            normalized.push(email);
        }
    }
    normalized
}

pub fn hash_machine_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.trim().as_bytes());
    hex::encode(hasher.finalize())
}

pub fn create_machine_token() -> String {
    let mut bytes = [0_u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

pub fn read_env_file(path: &Path) -> Result<BTreeMap<String, String>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let mut values = BTreeMap::new();
    for line in fs::read_to_string(path)?.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            let key = key.trim();
            if !key.is_empty()
                && key.chars().enumerate().all(|(idx, ch)| {
                    if idx == 0 {
                        ch.is_ascii_alphabetic() || ch == '_'
                    } else {
                        ch.is_ascii_alphanumeric() || ch == '_'
                    }
                })
            {
                values.insert(key.to_string(), value.to_string());
            }
        }
    }
    Ok(values)
}

pub fn write_env_file(path: &Path, values: &BTreeMap<String, String>) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lines = values
        .iter()
        .filter(|(_, value)| !value.is_empty())
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>();
    fs::write(path, format!("{}\n", lines.join("\n")))?;
    Ok(())
}

pub fn validate_emailish(value: Option<&str>, field_name: &str) -> Result<String> {
    let normalized = normalize_email(value);
    match normalized {
        Some(email) if email.contains('@') => Ok(email),
        Some(_) | None => bail!("{field_name} is required"),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        bounded_kube_name, create_machine_token, display_name_from_id, hash_machine_token,
        normalize_emails, read_env_file, sanitize_name, slugify, write_env_file,
    };
    use std::collections::BTreeMap;
    use tempfile::tempdir;

    #[test]
    fn slugify_collapses_noise() {
        assert_eq!(slugify(" Paul 2!!! "), "paul-2");
    }

    #[test]
    fn display_name_drops_suffix() {
        assert_eq!(display_name_from_id("paul-finite"), "Paul");
        assert_eq!(display_name_from_id("paul-finite-2"), "Paul Finite 2");
    }

    #[test]
    fn sanitize_name_replaces_underscores_and_dots() {
        assert_eq!(sanitize_name("paul.finite_2"), "paul-finite-2");
    }

    #[test]
    fn bounded_kube_name_leaves_short_names_alone() {
        assert_eq!(
            bounded_kube_name("skyler-finite-john1.finite.vip", "oauth2-proxy"),
            "skyler-finite-john1-finite-vip-oauth2-proxy"
        );
    }

    #[test]
    fn bounded_kube_name_truncates_and_hashes_long_names() {
        let name = bounded_kube_name(
            "jeremy-ani-art-academies-dashboard.trf.finite.computer",
            "oauth2-proxy",
        );
        assert!(name.len() <= 63);
        assert!(name.ends_with("-oauth2-proxy"));
        assert_ne!(
            name,
            "jeremy-ani-art-academies-dashboard-trf-finite-computer-oauth2-proxy"
        );
    }

    #[test]
    fn normalize_emails_dedupes() {
        let values = vec![
            "Paul@Finite.Vip".to_string(),
            " paul@finite.vip ".to_string(),
            "austin@finite.vip".to_string(),
        ];
        assert_eq!(
            normalize_emails(&values),
            vec![
                "paul@finite.vip".to_string(),
                "austin@finite.vip".to_string()
            ]
        );
    }

    #[test]
    fn token_hash_is_stable() {
        assert_eq!(
            hash_machine_token("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn machine_token_is_non_empty() {
        assert!(create_machine_token().len() >= 32);
    }

    #[test]
    fn env_files_round_trip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.env");
        let mut values = BTreeMap::new();
        values.insert("A".to_string(), "1".to_string());
        values.insert("EMPTY".to_string(), String::new());
        write_env_file(&path, &values).unwrap();
        let read = read_env_file(&path).unwrap();
        assert_eq!(read.get("A").map(String::as_str), Some("1"));
        assert!(!read.contains_key("EMPTY"));
    }
}
