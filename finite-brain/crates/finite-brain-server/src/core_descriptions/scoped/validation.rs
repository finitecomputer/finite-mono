use super::*;

pub(crate) fn validate_scoped_response(
    request: &ScopedLookupRequest,
    response: ScopedCoreResponse,
) -> Result<ScopedCoreResponse, CoreLookupFailure> {
    request.validate()?;
    let invalid = || CoreLookupFailure::Unavailable("malformed or unbound v3 response".to_owned());
    if response.version != DESCRIPTIONS_VERSION_V3
        || response.brain_id != request.brain_id
        || !timestamp(&response.checked_at)
        || response.checked_at.len() > 40
    {
        return Err(invalid());
    }
    let asked: BTreeSet<_> = request.keys.iter().collect();
    let answered: BTreeSet<_> = response.results.iter().map(|r| &r.public_key_hex).collect();
    if asked != answered || response.results.len() != asked.len() {
        return Err(invalid());
    }
    for row in &response.results {
        if !row_well_formed(&row.legacy()) || !provenance_valid(row) {
            return Err(invalid());
        }
        if !request.participating_keys.contains(&row.public_key_hex) {
            let used = row
                .operator_record
                .as_ref()
                .is_some_and(|r| r.state == "used");
            let contact_missing = row.state == "unknown"
                && row.operator_record.as_ref().is_some_and(|r| {
                    r.state == "suppressed" && r.reason.as_deref() == Some("contactUnavailable")
                });
            if !used && row.state != "notShared" && !contact_missing {
                return Err(invalid());
            }
        }
    }
    Ok(response)
}

fn evidence_time(value: &str) -> bool {
    (20..=40).contains(&value.len())
        && value.ends_with('Z')
        && value.as_bytes().get(10) == Some(&b'T')
        && value.as_bytes().get(17..19) != Some(b"60")
        && time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
            .is_ok_and(|t| t.year() >= 1)
}

fn annotation_valid(annotation: &OperatorAnnotation) -> bool {
    matches!(
        annotation.kind.as_str(),
        "historical_identity" | "invitation_key"
    ) && !annotation.label.is_empty()
        && annotation.label.len() <= 120
        && annotation.label.trim() == annotation.label
        && annotation.label.bytes().all(|b| (0x20..=0x7e).contains(&b))
        && evidence_time(&annotation.observed_at)
}

fn provenance_valid(row: &ScopedCoreDescription) -> bool {
    let sourced = row
        .source
        .as_ref()
        .is_some_and(|s| s.kind == OPERATOR_SOURCE);
    let Some(record) = &row.operator_record else {
        return !sourced && row.operator_annotation.is_none();
    };
    if !text(&record.revision, MAX_TOKEN_BYTES) || !evidence_time(&record.observed_at) {
        return false;
    }
    match record.state.as_str() {
        "used" => {
            if record.reason.is_some() {
                return false;
            }
            match (&row.operator_annotation, sourced) {
                (Some(annotation), false) => {
                    row.state == "notShared"
                        && annotation_valid(annotation)
                        && annotation.observed_at == record.observed_at
                }
                (None, true) => {
                    row.state == "resolved"
                        && row.source.as_ref().is_some_and(|source| {
                            source.observed_at == record.observed_at
                                && source.revision == record.revision
                        })
                        && row
                            .responsible_account
                            .as_ref()
                            .is_none_or(|account| account.human_public_keys_hex.is_empty())
                }
                _ => false,
            }
        }
        "suppressed" => {
            !sourced
                && row.operator_annotation.is_none()
                && matches!(
                    record.reason.as_deref(),
                    Some("normalResultPrecedence" | "contactUnavailable")
                )
        }
        "conflict" => {
            !sourced
                && row.operator_annotation.is_none()
                && matches!(
                    record.reason.as_deref(),
                    Some(
                        "accountBindingChanged"
                            | "identityEvidenceConflict"
                            | "agentBindingChanged"
                            | "annotationSuperseded"
                    )
                )
        }
        _ => false,
    }
}
