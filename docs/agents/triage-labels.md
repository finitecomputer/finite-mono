# Triage in Linear

Matt Pocock's triage roles are mapped below. Delivery status, work kind, and
triage role answer different questions. In Review can describe either a plan
review or an implementation review; the work kind identifies which one.

## Role mapping

| Skill role | Linear representation | Meaning |
| --- | --- | --- |
| `needs-triage` | `needs-triage` label | A maintainer must evaluate the request |
| `needs-info` | `needs-info` label | Specific information is missing |
| `ready-for-agent` | `ready-for-agent` label | The approved next task is sufficiently specified for an agent |
| `ready-for-human` | `ready-for-human` label | The specified next task needs human judgment or implementation |
| `wontfix` | Canceled status and a recorded reason | The request will not be acted on |

The four labels belong to one mutually exclusive group. When changing roles,
replace the previous role label and preserve unrelated labels. Conflicting
role labels need maintainer resolution. Canceled issues need no active role
label; explain the reason rather than assigning a `wontfix` label. Use the
native Duplicate state and relationship for duplicate requests.

For categories, map `bug` to **Bug**, and `enhancement` to **Feature** or
**Improvement**, according to the request. Keep an existing matching category;
use Feature for new behavior and Improvement for a change to existing behavior.
An active triaged request has one category and one role. A request with only
area labels still lacks triage classification.

Readiness does not schedule work or establish design approval. Preserve the
delivery status unless the requested action changes it. Select agent work
from open Build issues marked `ready-for-agent`, selected for work in Todo,
whose blockers are complete. Show specified Backlog work separately. An
Engineering Plan in review or Done is not a second implementation assignment.
Clear active triage-role labels when completing an issue.

## Existing labels and rollout

The target work-kind labels are **Engineering Plan** and **Build**. Consolidate
the overlapping **Design** and **Decision** labels into Engineering Plan after
checking the affected issues. Until that migration is complete, recognize all
three planning labels when reading existing work. Classify by the issue's
actual deliverable if its labels are missing or inconsistent.

Keep affected-area labels such as Sites, Chat, Agent Runtime, and Auth.
**Review** currently means review tooling; the proposed name is **Review
Tooling**. **Tiny Sprint** is an import marker; retire it only after checking
the views that use it. Neither label is a triage role.

The draft skill PR proposes this setup; it does not create labels or change
Linear settings. Inspect the team's live labels before applying the mapping.
If a required label or group is missing, draft the intended classification and
report the setup gap. Do not invent an equivalent from Backlog, Todo, In Review,
or unrelated labels. Workspace setup is a separate, explicitly requested task.
