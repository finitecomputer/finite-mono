# Domain documentation

Domain-modeling guidance is adapted from Matt Pocock's
[domain-modeling](https://github.com/mattpocock/skills/tree/3cca18b368ae95cdbdebbff572ccafa662551015/skills/engineering/domain-modeling)
under the [MIT license](../../.agents/skills/LICENSE).

AGENTS.md owns enduring engineering principles and repo operating rules.
Linear owns evolving terminology, architectural decisions, specifications, and
work. Code, tests, and executable configuration establish implemented behavior.
Apply principles relevant to the change; keep one authoritative copy of each.

## Find the current context

1. Read the relevant Linear issue, comments, and linked plans. Follow only the
   component decisions and review concerns relevant to the work.
2. Find the engineering glossary and decision index through the issue's links
   or the Finite Engineering team's documents. Check their scope and sources;
   a matching title alone does not establish authority.
3. Until each record has migrated, use `CONTEXT-MAP.md` to find the component's
   `CONTEXT.md`, and read relevant root or component ADRs and runbooks.
4. If the sources disagree, identify the conflicting statements and their
   scopes. Resolve the affected decision with its owner before declaring the
   work ready. A newer timestamp alone does not supersede an accepted contract.

The initial Linear glossary and decision index are proposed in the draft PR;
their publication has not been verified. Retained repo records remain the
source for unmigrated material. If Linear is unavailable, state what remains
unverified and keep new planning in the conversation. Do not claim that repo
context proves the latest approval state.

## Domain modeling during a grill

Challenge terms that conflict with the glossary. Ask which established concept
the user means when a word is ambiguous. Test the relationships with concrete
scenarios and compare claimed behavior with the code.

When a term is resolved, record it in the scoped Linear glossary. A definition
is one or two sentences about what the concept is; include terms to avoid when
they prevent ambiguity. Keep implementation plans in the specification.
Group definitions by context, since a word such as Project can mean different
things in different products or in Linear itself.

Offer an architectural decision record only when all three conditions hold:

- Changing the decision later has a meaningful cost.
- A future reader would need its rationale.
- The team chose between real alternatives.

Use an existing plan or decision record when it already owns the choice. Create
a separate Linear document only when the decision needs an independent life.
Record the decision, rationale, scope, and approval source. Keep proposed,
accepted, and superseded decisions distinct. The decision index links to the
record; it does not copy its contents. An engineer's recorded concern is input
to review, not evidence that they approved the current proposal.

Capture settled terms and decisions as the session proceeds within the user's
authorized scope. In draft-only sessions, keep proposed edits in the
conversation. Changes to shared principles or another component's contract
remain proposals until the required owners accept them; see
[issue-tracker.md](issue-tracker.md) for approval rules. Publishing documentation
does not authorize implementation.

## Rejected and deferred requests

During triage, search the decision index, relevant closed Linear issues, and
any retained `.out-of-scope/` records by concept. Record a rejected enhancement's
reason and related requests in its existing Linear decision record, or create
one if the rationale deserves an independent record. Index the rejection so a
later request can find it. Keep temporary deferral distinct from rejection.
Already-implemented requests need a link to the implementation, not a rejection
record. Reconsideration updates the current decision while preserving the
source of the previous decision.

## Migrate existing records

For each authorized migration, reconcile the source, publish the accepted
content in Linear, and read it back. Record the replacement URL and source
revision, update incoming references, then delete or shorten the replaced repo
material. Partial migration removes only the covered content. Keep operational
instructions and contracts still consumed by code, tests, or production until
their replacement is verified. Keep new planning out of repo-local PRDs, ADRs,
glossaries, and work logs.

FiniteBrain continues to hold the existing org wiki, postmortems, and runbooks
described in AGENTS.md. Follow its read flow when such material is referenced.
Link relevant evidence from Linear; moving it or maintaining a second copy is
separate work.
