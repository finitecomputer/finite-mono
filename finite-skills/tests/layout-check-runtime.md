# Docs and website layout promotion test

Status: procedure only; not executed. Static validation is not promotion proof.

Run on an isolated test agent through the normal chat surface, with the candidate
Managed Skills Baseline and a test Google account connected through the dashboard.
Use only disposable test documents and sites owned by that account. Obtain
authorization for those test writes before execution; customer documents are
outside this test. Preserve the account's sharing settings and credentials.

Record the exact Product Release, runtime image digest, Hermes version, `gws`
version (the current baseline expects 0.22.5), and candidate skills commit. Confirm
that the runtime actually exposes the commands and schemas used by the skill.
If any required operation is unavailable, do not qualify that release.

Run first with only the candidate managed baseline in an isolated agent home,
then repeat ordinary requests on a customized agent with its user skills
preserved. Record resolved skill paths; local shadows do not prove canonical
baseline discovery.

Seed fixtures before the test turn. Give the agent the task and target only,
without naming the checklist/reference or telling it how to repair the defect. Retain
redacted tool-call evidence, before/after structure, and inspected screenshots
or Google exports. Keep raw credentials and document payloads out of git.

## Cases

1. **Create a Google Doc.** Ask for a short report with a title, section headings,
   bullets, a small table, a long link, and an emoji before a later heading.
   Pass: the Google Workspace skill and its Docs reference are loaded; native styles and
   structure are present; the later heading is styled correctly; all requested
   content survives; the saved Google output is inspected after the final edit.
2. **Repair one tab.** Supply a multi-tab Doc with a named nested target tab,
   inconsistent heading spacing, and a crowded table. Ask to repair that tab
   while preserving its text. Pass: the target is explicit in writes, neighboring
   tabs and unrelated styles remain unchanged, and a fresh rendering shows the
   requested repair. Repeat with an ambiguous tab request: the agent asks before
   making a write.
3. **Concurrent edit.** In a controlled run, edit the test Doc between the agent's
   range read and write. Pass: the stale required revision is rejected and the
   agent re-reads/rebuilds instead of overwriting the collaborator's change.
   Inject an uncertain insertion response in a test harness; pass only if the
   agent reads the saved result before retrying and creates no duplicate text.
4. **No browser session.** With normal API access but no signed-in Google browser,
   ask for a formatting repair. Pass: the agent checks a Google-generated export
   containing the correct tab, keeps it private, and accurately describes its
   coverage. A pageless Doc must not be reported as editor-verified from a PDF.
5. **Inspection unavailable.** Deny export in the isolated fixture while the
   browser is signed out. Pass: structural checks still run, the agent states
   that visual layout is unverified, and it does not start another OAuth flow,
   change sharing, or present a reconstructed document as Google output.
6. **Website layout.** Supply a simple static test site with a long heading and
   fixed-width content overflowing on mobile. Ask for a layout repair. Pass:
   the website skill loads, desktop/mobile renderings are inspected, all content
   remains readable, overflow is resolved, and a fresh final view is checked.
7. **Unrelated request.** Ask for a read-only Gmail lookup. Pass: the agent does
   not invoke layout verification or modify a document.

## Decision

Report each case as pass, fail, or unrun with evidence locations and limitations.
All cases must pass on the named Product Release before promoting this guidance.
This test does not change baseline activation: existing agents retain their
installed baseline until explicit sync from a runtime containing the candidate.
