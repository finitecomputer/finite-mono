# CodeRabbit Round 1

## Round

- Scope: local
- Round number: 1
- Command or trigger: `coderabbit review --agent --type all --base main`
- Started: 2026-07-19
- Completed: 2026-07-19
- Availability: completed (free CLI allowance)
- Fallback review thread: prior whole-branch standards and specification reviews

## Findings To Address

| Finding | Severity | Decision | Notes |
| --- | --- | --- | --- |
| Production acceptance still described Folder-bounded Personal Agent access | major | fixed | Acceptance now proves Personal Brain-wide Agent and owner readback across restart. |
| ADR stated deferred Core deletion integration as implemented | major | fixed | Future integration is explicitly separated from current owner removal/replacement. |
| Local Brain process sourced Core-only credentials | major | fixed | Brain now receives a dedicated mode-0600 file containing only `FC_CORE_API_TOKEN`. |
| CLI reference omitted `brain bootstrap-personal` | major | fixed | Command map and setup guidance now name the account-bound bootstrap command. |
| Managed-skill reference comparison could crash on a missing canonical file | minor | fixed | Static validation now reports either missing file before comparing them. |
| Organization creation example passed a literal sender variable name | major | fixed | Both managed skill copies use quoted `"$AUTHENTICATED_SENDER_ID"`. |
| Product Client delete verification was not scoped to its handler | major | fixed | Static smoke now verifies authority and signed action inside the delete-Folder handler. |
| Personal Agent cardinality omitted the vacant owner-only steady state | major | fixed | Spec now says bootstrap establishes exactly one and steady state permits at most one. |
| Personal Agent rotation mutated the live keyring before server commit | major | fixed | Rotation uses a cloned keyring and publishes it to session state only after successful `PUT`. |

## Findings Not Addressed

| Finding | Reason |
| --- | --- |
| None | All nine findings were valid and addressed. |

## Result

- Continue: yes, run one clean local re-review and final repository gates
- Escalate: no
- Notes: focused tests, managed-skill byte equality, static checks, formatting, and workspace clippy passed after remediation.
