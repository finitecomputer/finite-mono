//! `fbrain access list`: one fresh server-backed named access report for one
//! exact Brain ID. JSON and human output render the same merged report.

use std::collections::BTreeSet;
use std::io::Write;

use serde::Deserialize;
use serde_json::Value;

use crate::signer::load_signer;
use crate::{CliEnvironment, CliError, signed_json_request_with_response_limit, write_json};

pub(crate) const ACCESS_REPORT_VERSION: &str = "finite-brain-access-report-v1";
const PAGE_LIMIT: usize = 100;
const MAX_REPORT_PAGES: usize = 100;
const MAX_REPORT_ATTEMPTS: usize = 2;
const MAX_PAGE_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

/// Fields every page of one report must repeat exactly.
const PAGE_INVARIANTS: [&str; 9] = [
    "brainId",
    "brainKind",
    "brainName",
    "authorityFingerprint",
    "actingKey",
    "coverage",
    "currentAccessComplete",
    "totals",
    "incomingMounts",
];

/// Fetch every page under one authority fingerprint. If the Brain changes
/// between pages (the server answers 409, or a page carries a different
/// fingerprint) the whole report is re-read; nothing older is substituted.
pub(crate) fn fetch_access_report(
    env: &CliEnvironment,
    args: &[String],
    brain_id: &str,
) -> Result<Value, CliError> {
    let own_npub = load_signer(env)?.npub;
    for _ in 0..MAX_REPORT_ATTEMPTS {
        if let Some(report) = fetch_consistent_pages(env, args, brain_id, &own_npub)? {
            return Ok(report);
        }
    }
    Err(CliError::InvalidInput(
        "Brain access changed while the access report was being read; retry `fbrain access list`"
            .to_owned(),
    ))
}

fn fetch_consistent_pages(
    env: &CliEnvironment,
    args: &[String],
    brain_id: &str,
    own_npub: &str,
) -> Result<Option<Value>, CliError> {
    let mut merged: Option<Value> = None;
    let mut after: Option<String> = None;
    let mut last_npub: Option<String> = None;
    let mut seen = BTreeSet::new();
    for page_index in 0..MAX_REPORT_PAGES {
        let mut path = format!("/v1/brains/{brain_id}/access-report?limit={PAGE_LIMIT}");
        if let Some(cursor) = &after {
            path.push_str(&format!("&after={cursor}"));
        }
        let page = match signed_json_request_with_response_limit(
            env,
            args,
            "GET",
            &path,
            None,
            MAX_PAGE_RESPONSE_BYTES,
        ) {
            Ok(page) => page,
            // Authority drift on any page restarts the whole bounded report.
            Err(CliError::HttpStatus { status: 409, .. }) => return Ok(None),
            Err(error) => return Err(unsupported_report_server(error, brain_id)),
        };
        if page.get("version").and_then(Value::as_str) != Some(ACCESS_REPORT_VERSION) {
            return Err(CliError::Unsupported(format!(
                "the Brain server returned an unrecognized access report; expected {ACCESS_REPORT_VERSION}. Upgrade fbrain or the Brain server"
            )));
        }
        validate_page(&page, brain_id, own_npub, after.as_deref())?;
        if let Some(report) = merged.as_ref() {
            if report["authorityFingerprint"] != page["authorityFingerprint"] {
                return Ok(None);
            }
            if let Some(field) = PAGE_INVARIANTS
                .iter()
                .find(|field| report[**field] != page[**field])
            {
                return Err(malformed(&format!("pages disagree on {field}")));
            }
        }
        for row in page["identities"].as_array().into_iter().flatten() {
            let npub = row["npub"].as_str().unwrap_or_default().to_owned();
            if last_npub.as_ref().is_some_and(|last| npub <= *last) || !seen.insert(npub.clone()) {
                return Err(malformed(
                    "identities are not strictly ordered and unique across pages",
                ));
            }
            last_npub = Some(npub);
        }
        let next = page["page"]["next"].as_str().map(ToOwned::to_owned);
        match merged.as_mut() {
            None => merged = Some(page),
            Some(report) => merge_page(report, page),
        }
        match next {
            Some(cursor) => after = Some(cursor),
            None => {
                let mut report = merged.take().unwrap_or(Value::Null);
                finish_report(&mut report, page_index + 1)?;
                return Ok(Some(report));
            }
        }
    }
    Err(CliError::InvalidInput(format!(
        "the access report exceeded {MAX_REPORT_PAGES} pages; it was not truncated. Narrow the Brain or contact the operator"
    )))
}

fn malformed(reason: &str) -> CliError {
    CliError::InvalidInput(format!(
        "the Brain server returned a malformed access report ({reason}); nothing was shown"
    ))
}

// Required v1 fields have no defaults: an omitted fact is not an empty or
// complete scope. The original Value is retained for forward-compatible output.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequiredPage {
    brain_kind: String,
    brain_name: String,
    #[serde(rename = "authoritySequence")]
    _authority_sequence: u64,
    checked_at: String,
    coverage: RequiredCoverage,
    current_access_complete: bool,
    totals: RequiredTotals,
    page: RequiredPagination,
    identities: Vec<RequiredIdentity>,
    directory: RequiredDirectory,
    incoming_mounts: Vec<RequiredIncomingMount>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequiredCoverage {
    members: RequiredScope,
    guests: RequiredScope,
    mounts: RequiredScope,
    current_grants: RequiredScope,
    access_history: RequiredScope,
}

#[derive(Deserialize)]
struct RequiredScope {
    state: String,
    reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequiredTotals {
    identities: usize,
    revocation_incomplete: usize,
    grants_missing: usize,
}

#[derive(Deserialize)]
struct RequiredPagination {
    limit: usize,
    returned: usize,
    #[serde(rename = "after")]
    _after: Option<String>,
    #[serde(rename = "next")]
    _next: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequiredDirectory {
    state: String,
    checked_keys: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequiredIncomingMount {
    mount_id: String,
    display_name: String,
    source_brain_id: String,
    source_folder_id: String,
    connection_status: String,
    participant_detail: String,
    #[serde(rename = "participants")]
    _participants: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequiredIncomingEntry {
    mount_id: String,
    display_name: String,
    source_brain_id: String,
    source_folder_id: String,
    mount_access: bool,
    current_grant: String,
    state: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequiredIdentity {
    npub: String,
    hex: String,
    brain_role: String,
    role_sources: Vec<String>,
    name: RequiredName,
    identity_type: RequiredType,
    folders: Vec<RequiredFolder>,
    missing_current_grants: Vec<String>,
    revocation_incomplete: Vec<String>,
    #[serde(default)]
    incoming_mounts: Vec<RequiredIncomingEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequiredName {
    state: String,
    display: String,
    source: Option<String>,
    checked_at: Option<String>,
    matched_key: Option<String>,
}

#[derive(Deserialize)]
struct RequiredType {
    value: String,
    label: String,
    evidence: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequiredFolder {
    folder_id: String,
    path: String,
    access_mode: String,
    key_version: u32,
    entitled: bool,
    entitlement_sources: Vec<String>,
    current_grant: String,
    state: String,
    grant: Option<RequiredGrant>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RequiredGrant {
    issued_by: String,
    issued_at: String,
    signed_audit: String,
}

fn validate_required_facts(page: &Value) -> Result<(), CliError> {
    let facts: RequiredPage = serde_json::from_value(page.clone())
        .map_err(|error| malformed(&format!("missing or mistyped required fields: {error}")))?;
    if !matches!(facts.brain_kind.as_str(), "personal" | "organization")
        || facts.brain_name.is_empty()
        || time::OffsetDateTime::parse(
            &facts.checked_at,
            &time::format_description::well_known::Rfc3339,
        )
        .is_err()
        || facts.page.limit == 0
        || facts.page.limit > PAGE_LIMIT
        || facts.page.returned != facts.identities.len()
        || facts.page.returned > facts.page.limit
        || facts.totals.identities < facts.identities.len()
        || !matches!(
            facts.directory.state.as_str(),
            "checked" | "notConfigured" | "notNeeded" | "unavailable" | "unsupported"
        )
        || facts.directory.checked_keys > facts.identities.len()
        || !matches!(
            page["actingKey"]["brainRole"].as_str(),
            Some("owner" | "personalAgent" | "admin")
        )
    {
        return Err(malformed("invalid report facts or page counts"));
    }
    let current = [
        &facts.coverage.members,
        &facts.coverage.guests,
        &facts.coverage.mounts,
        &facts.coverage.current_grants,
    ];
    for scope in current.into_iter().chain([&facts.coverage.access_history]) {
        if !matches!(
            scope.state.as_str(),
            "complete" | "unverified" | "unsupported"
        ) || (scope.state != "complete" && scope.reason.as_deref().is_none_or(str::is_empty))
        {
            return Err(malformed("invalid or unexplained coverage"));
        }
    }
    if facts.current_access_complete != current.iter().all(|scope| scope.state == "complete")
        || (facts.current_access_complete
            && facts
                .incoming_mounts
                .iter()
                .any(|mount| mount.connection_status == "active"))
    {
        return Err(malformed(
            "currentAccessComplete contradicts scope coverage",
        ));
    }
    for mount in &facts.incoming_mounts {
        if mount.mount_id.is_empty()
            || mount.display_name.is_empty()
            || mount.source_brain_id.is_empty()
            || mount.source_folder_id.is_empty()
            || !matches!(
                mount.connection_status.as_str(),
                "active" | "revoked" | "missing"
            )
            || !matches!(mount.participant_detail.as_str(), "complete" | "withheld")
        {
            return Err(malformed("invalid incoming Mount facts"));
        }
    }
    let mut page_revoked = 0;
    let mut page_missing = 0;
    for row in facts.identities {
        if row.npub.is_empty()
            || row.brain_role.is_empty()
            || row.role_sources.is_empty()
            || row.name.display.is_empty()
            || !matches!(
                row.name.state.as_str(),
                "verified" | "domainClaimed" | "storedNotRechecked" | "unknown"
            )
            || !matches!(
                row.identity_type.value.as_str(),
                "managedAgent" | "notConfirmed"
            )
            || row.identity_type.label.is_empty()
            || row.identity_type.evidence.is_empty()
        {
            return Err(malformed("missing identity facts"));
        }
        if matches!(row.name.state.as_str(), "verified" | "domainClaimed")
            && (row.name.source.as_deref().is_none_or(str::is_empty)
                || row.name.checked_at.as_deref().is_none_or(str::is_empty)
                || row.hex.is_empty()
                || row.name.matched_key.as_deref() != Some(row.hex.as_str()))
        {
            return Err(malformed("verified name is missing exact-key evidence"));
        }
        for mount in row.incoming_mounts {
            let expected = match (mount.mount_access, mount.current_grant.as_str()) {
                (true, "present") => "ready",
                (true, "missing") => "grantMissing",
                (false, "present" | "missing") => "mountAccessMissing",
                _ => return Err(malformed("invalid incoming Mount grant state")),
            };
            if mount.mount_id.is_empty()
                || mount.display_name.is_empty()
                || mount.source_brain_id.is_empty()
                || mount.source_folder_id.is_empty()
                || mount.state != expected
            {
                return Err(malformed("inconsistent incoming Mount state"));
            }
        }
        let mut missing = BTreeSet::new();
        let mut revoked = BTreeSet::new();
        let mut folders = BTreeSet::new();
        for folder in row.folders {
            let expected = match (folder.entitled, folder.current_grant.as_str()) {
                (true, "present") => "ready",
                (true, "missing") => "grantMissing",
                (false, "present") => "revocationIncomplete",
                _ => return Err(malformed("invalid Folder entitlement/grant facts")),
            };
            if folder.folder_id.is_empty()
                || !folders.insert(folder.folder_id.clone())
                || folder.path.is_empty()
                || !matches!(
                    folder.access_mode.as_str(),
                    "owner" | "admin_only" | "all_members" | "restricted"
                )
                || folder.key_version == 0
                || folder.state != expected
                || folder.entitled == folder.entitlement_sources.is_empty()
                || (folder.current_grant == "present") != folder.grant.is_some()
            {
                return Err(malformed("inconsistent Folder state"));
            }
            if let Some(grant) = folder.grant
                && (grant.issued_by.is_empty()
                    || grant.issued_at.is_empty()
                    || !matches!(
                        grant.signed_audit.as_str(),
                        "verified" | "mismatched" | "unreadable" | "notStored"
                    ))
            {
                return Err(malformed("invalid grant evidence"));
            }
            if expected == "grantMissing" {
                missing.insert(folder.folder_id.clone());
            }
            if expected == "revocationIncomplete" {
                revoked.insert(folder.folder_id);
            }
        }
        if missing.len() != row.missing_current_grants.len()
            || revoked.len() != row.revocation_incomplete.len()
            || missing
                != row
                    .missing_current_grants
                    .into_iter()
                    .collect::<BTreeSet<_>>()
            || revoked
                != row
                    .revocation_incomplete
                    .into_iter()
                    .collect::<BTreeSet<_>>()
        {
            return Err(malformed("identity gap lists disagree with Folder facts"));
        }
        page_missing += missing.len();
        page_revoked += revoked.len();
    }
    if page_missing > facts.totals.grants_missing
        || page_revoked > facts.totals.revocation_incomplete
    {
        return Err(malformed("page gaps exceed report totals"));
    }
    Ok(())
}

/// One page must be for exactly this Brain and this calling key, carry a
/// well-formed fingerprint, echo the cursor it was asked for, and end with a
/// cursor bound to its own fingerprint and last row.
fn validate_page(
    page: &Value,
    brain_id: &str,
    own_npub: &str,
    after: Option<&str>,
) -> Result<(), CliError> {
    validate_required_facts(page)?;
    if page["brainId"].as_str() != Some(brain_id) {
        return Err(malformed("it is for a different Brain"));
    }
    if page["actingKey"]["npub"].as_str() != Some(own_npub) {
        return Err(malformed("it was checked for a different calling key"));
    }
    let fingerprint = page["authorityFingerprint"].as_str().unwrap_or_default();
    if fingerprint.len() != 64
        || !fingerprint
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        return Err(malformed("missing or invalid authority fingerprint"));
    }
    if page["page"]["after"].as_str() != after {
        return Err(malformed(
            "the page does not continue from the requested cursor",
        ));
    }
    let Some(rows) = page["identities"].as_array() else {
        return Err(malformed("identities missing"));
    };
    if rows
        .iter()
        .any(|row| row["npub"].as_str().is_none_or(str::is_empty))
    {
        return Err(malformed("an identity has no key"));
    }
    if let Some(next) = page["page"]["next"].as_str() {
        let last = rows.last().and_then(|row| row["npub"].as_str());
        if last.is_none_or(|last| next != format!("{fingerprint}.{last}")) {
            return Err(malformed("the next-page cursor does not match this page"));
        }
    }
    Ok(())
}

/// A 404 that is the server's unknown-route answer means an older Brain
/// server. Fail clearly instead of falling back to a weaker metadata view.
fn unsupported_report_server(error: CliError, brain_id: &str) -> CliError {
    match error {
        CliError::HttpStatus { status: 404, body } if body.contains("route not found") => {
            unsupported(brain_id)
        }
        CliError::HttpStatus { status: 405, .. } => unsupported(brain_id),
        other => other,
    }
}

fn unsupported(brain_id: &str) -> CliError {
    CliError::Unsupported(format!(
        "this Brain server does not provide the named access report (GET /v1/brains/{brain_id}/access-report). Upgrade the Brain server; no weaker metadata summary was substituted. `fbrain access summary` shows the older metadata view, which names no identities and proves no coverage"
    ))
}

fn merge_page(report: &mut Value, page: Value) {
    if let (Some(rows), Some(more)) = (
        report["identities"].as_array_mut(),
        page["identities"].as_array(),
    ) {
        rows.extend(more.iter().cloned());
    }
    let worse = directory_rank(&page["directory"]) > directory_rank(&report["directory"]);
    let checked = report["directory"]["checkedKeys"].as_u64().unwrap_or(0)
        + page["directory"]["checkedKeys"].as_u64().unwrap_or(0);
    if worse {
        report["directory"] = page["directory"].clone();
    }
    report["directory"]["checkedKeys"] = checked.into();
}

fn directory_rank(directory: &Value) -> u8 {
    match directory["state"].as_str() {
        Some("notNeeded") => 0,
        Some("checked") => 1,
        Some("notConfigured") => 2,
        _ => 3,
    }
}

fn finish_report(report: &mut Value, pages: usize) -> Result<(), CliError> {
    let rows = report["identities"].as_array().map_or(0, Vec::len);
    let total = report["totals"]["identities"]
        .as_u64()
        .ok_or_else(|| malformed("missing identity total"))? as usize;
    if rows != total {
        return Err(CliError::InvalidInput(format!(
            "the access report returned {rows} of {total} identities; refusing to show a partial report"
        )));
    }
    let identities = report["identities"]
        .as_array()
        .ok_or_else(|| malformed("missing identities"))?;
    for (total, field) in [
        ("grantsMissing", "missingCurrentGrants"),
        ("revocationIncomplete", "revocationIncomplete"),
    ] {
        let actual = identities
            .iter()
            .map(|row| row[field].as_array().map_or(0, Vec::len))
            .sum::<usize>();
        if report["totals"][total].as_u64() != Some(actual as u64) {
            return Err(malformed("report gap totals disagree with identity facts"));
        }
    }
    if let Some(object) = report.as_object_mut() {
        object.remove("page");
        object.insert("pages".to_owned(), pages.into());
    }
    Ok(())
}

pub(crate) fn write_access_report<W: Write>(
    output: &mut W,
    json: bool,
    report: &Value,
) -> Result<(), CliError> {
    if json {
        return write_json(output, report);
    }
    let text = |value: &Value| {
        value
            .as_str()
            .unwrap_or("?")
            .chars()
            .map(|ch| if ch.is_control() || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') { ' ' } else { ch })
            .collect::<String>()
    };
    writeln!(
        output,
        "Access report for Brain {} ({})",
        text(&report["brainId"]),
        text(&report["brainName"])
    )?;
    writeln!(
        output,
        "Checked {} at authority sequence {} (authority {}) as {} ({})",
        text(&report["checkedAt"]),
        report["authoritySequence"],
        text(&report["authorityFingerprint"]),
        text(&report["actingKey"]["npub"]),
        text(&report["actingKey"]["brainRole"])
    )?;
    let coverage = &report["coverage"];
    writeln!(
        output,
        "Coverage: members {}; guests {}; mounts {}; current grants {}; access history {}",
        text(&coverage["members"]["state"]),
        text(&coverage["guests"]["state"]),
        text(&coverage["mounts"]["state"]),
        text(&coverage["currentGrants"]["state"]),
        text(&coverage["accessHistory"]["state"]),
    )?;
    for scope in [
        "members",
        "guests",
        "mounts",
        "currentGrants",
        "accessHistory",
    ] {
        if coverage[scope]["state"] != "complete" && coverage[scope]["reason"].is_string() {
            writeln!(output, "  {scope}: {}", text(&coverage[scope]["reason"]))?;
        }
    }
    writeln!(
        output,
        "Current access: {}",
        if report["currentAccessComplete"] == true {
            "complete"
        } else {
            "NOT complete; see coverage above"
        }
    )?;
    let directory = &report["directory"];
    match directory["reason"].as_str() {
        Some(_) => writeln!(
            output,
            "Names: Identity Directory {} ({})",
            text(&directory["state"]),
            text(&directory["reason"])
        )?,
        None => writeln!(
            output,
            "Names: Identity Directory {}",
            text(&directory["state"])
        )?,
    }
    writeln!(
        output,
        "{} identities; {} revocation incomplete; {} current grants missing",
        report["totals"]["identities"],
        report["totals"]["revocationIncomplete"],
        report["totals"]["grantsMissing"]
    )?;
    for mount in report["incomingMounts"].as_array().into_iter().flatten() {
        writeln!(
            output,
            "Mounted in: {} from Brain {} Folder {} (connection {}; {} participant(s), detail {})",
            text(&mount["displayName"]),
            text(&mount["sourceBrainId"]),
            text(&mount["sourceFolderId"]),
            text(&mount["connectionStatus"]),
            mount["participants"],
            text(&mount["participantDetail"]),
        )?;
    }
    for row in report["identities"].as_array().into_iter().flatten() {
        write_identity(output, row)?;
    }
    writeln!(output)?;
    writeln!(
        output,
        "Names are labels, not authority: act only on exact keys. A present current grant means a wrapped key exists; it does not prove the key decrypted it."
    )?;
    Ok(())
}

fn write_identity<W: Write>(output: &mut W, row: &Value) -> Result<(), CliError> {
    let text = |value: &Value| {
        value
            .as_str()
            .unwrap_or("?")
            .chars()
            .map(|ch| if ch.is_control() || matches!(ch, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') { ' ' } else { ch })
            .collect::<String>()
    };
    writeln!(output)?;
    writeln!(
        output,
        "{} [{}]",
        text(&row["name"]["display"]),
        text(&row["identityType"]["label"])
    )?;
    writeln!(output, "  key: {}", text(&row["npub"]))?;
    let sources = row["roleSources"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|value| value.is_string())
        .map(text)
        .collect::<Vec<_>>()
        .join(", ");
    writeln!(output, "  role: {} ({sources})", text(&row["brainRole"]))?;
    let name = &row["name"];
    let evidence = match name["state"].as_str() {
        Some("verified") => format!(
            "verified for this exact key via {} at {}",
            text(&name["source"]),
            text(&name["checkedAt"])
        ),
        Some("domainClaimed") => format!(
            "domain claims this exact key via {} at {}; not independently verified",
            text(&name["source"]),
            text(&name["checkedAt"])
        ),
        Some("storedNotRechecked") => format!(
            "Stored name, not rechecked{}",
            name["storedVerifiedAt"]
                .as_str()
                .map(|_| format!(" (stored {})", text(&name["storedVerifiedAt"])))
                .unwrap_or_default()
        ),
        _ => format!(
            "unknown{}",
            name["reason"]
                .as_str()
                .map(|_| format!(": {}", text(&name["reason"])))
                .unwrap_or_default()
        ),
    };
    writeln!(output, "  name: {evidence}")?;
    writeln!(output, "  type: {}", text(&row["identityType"]["evidence"]))?;
    for folder in row["folders"].as_array().into_iter().flatten() {
        let state = match folder["state"].as_str() {
            Some("ready") => "current grant present".to_owned(),
            Some("grantMissing") => "entitled; current grant missing".to_owned(),
            Some("revocationIncomplete") => {
                "Revocation incomplete: current grant without entitlement".to_owned()
            }
            other => other.unwrap_or("?").to_owned(),
        };
        let sources = folder["entitlementSources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|value| value.is_string())
            .map(text)
            .collect::<Vec<_>>()
            .join(", ");
        let audit = folder["grant"]["signedAudit"]
            .as_str()
            .map(|audit| format!("; signed audit {audit}"))
            .unwrap_or_default();
        writeln!(
            output,
            "  - {} ({}, key v{}): {state} [{sources}]{audit}",
            text(&folder["path"]),
            text(&folder["accessMode"]),
            folder["keyVersion"]
        )?;
        if let Some(grant) = folder.get("grant").filter(|grant| grant.is_object()) {
            writeln!(
                output,
                "    grant issued by {} at {} (stored provenance)",
                text(&grant["issuedBy"]),
                text(&grant["issuedAt"])
            )?;
            if grant["signedAuditActor"].is_string() {
                writeln!(
                    output,
                    "    audit signer {} at {} ({})",
                    text(&grant["signedAuditActor"]),
                    text(&grant["signedAuditAt"]),
                    text(&grant["signedAudit"])
                )?;
            }
            if grant["provenance"]["origin"].is_string() {
                writeln!(
                    output,
                    "    origin {}",
                    text(&grant["provenance"]["origin"])
                )?;
            }
        }
    }
    for mount in row["incomingMounts"].as_array().into_iter().flatten() {
        writeln!(
            output,
            "  - mounted {} from {}/{}: {} (Mount access {}, current grant {})",
            text(&mount["displayName"]),
            text(&mount["sourceBrainId"]),
            text(&mount["sourceFolderId"]),
            text(&mount["state"]),
            if mount["mountAccess"] == true {
                "present"
            } else {
                "missing"
            },
            text(&mount["currentGrant"]),
        )?;
    }
    Ok(())
}
