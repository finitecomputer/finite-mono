# Google Docs formatting

Use the connected Google account through the tools described by
`google-workspace-finite`. Its bundled Python wrapper reads Docs only; use the
existing `gws` Docs write capability for edits. Inspect the installed command's
help and request schema before constructing requests. Keep credentials and
sharing unchanged. A missing capability is a limitation to report, not a reason
to substitute a different product or start another authentication flow.

## Read before editing

Read the target document with tab content included. Resolve the intended tab,
including nested tabs, from the user's request and the document structure;
specify that tab in writes. When the intended tab is ambiguous, ask. Preserve
the existing template, page or pageless mode, and formatting outside the task.

For edits based on read ranges, use the read revision as a required revision
in the write control. If it has changed, read again and rebuild the edit. If a
write times out, inspect the saved result before retrying an insertion so the
document does not acquire duplicate text.

## Use native structure

- Apply named title and heading paragraph styles for the hierarchy. Style body
  text consistently with the existing document or template.
- Use native bullets and numbering for lists, paragraph spacing for vertical
  rhythm, and real table cells for tabular data.
- Apply inline emphasis and links to the intended text ranges. Plain text
  insertion does not turn Markdown markers into formatted headings or bold
  text; convert markup to native styles when formatted prose is intended.
  Preserve literal markup in examples or code.
- Use a small set of consistent spacing and indentation choices. Avoid padding
  with repeated spaces or blank paragraphs to position text.
- Fit tables and images to the usable width. Keep body text readable when
  repairing crowded content; preserve all cells and words.

## Keep ranges correct

Docs indices count UTF-16 code units. Emoji can occupy more than one code unit;
derive ranges from the API structure rather than counting displayed characters.
Requests in a batch execute in order, and content changes can shift later
indices. For a simple implementation, insert the content, read it back, then
apply styles to those observed ranges. Re-read after table or list operations
that change structure before styling subsequent ranges.

Use narrow style field masks so a heading or spacing fix preserves unrelated
properties. Apply changes only to the requested ranges and tab. Check the API
schema for the specific table, list, paragraph, or text operation being used.

## Verify the actual Google document

Read back the saved document and confirm that its text, named paragraph styles,
list structure, table contents, and targeted tab match the intended result.
Then inspect its appearance:

1. Prefer the authenticated Google Docs view when available. Confirm the latest
   edit is saved and inspect the affected sections and surrounding content.
2. If the browser has no Google session, use the existing connection's supported
   Drive export capability to obtain a PDF of the saved Google Doc. Render and
   inspect it with available local tools. Keep the export private.
3. A PDF checks Google's exported page layout, not the pageless editor. State
   that distinction when it matters. Verify that the export includes the target
   tab and content before using it as evidence.

Inspect every page of a short document. For a long document, inspect all changed
sections, affected page boundaries, and downstream pages where content moved;
state the coverage. Check for stranded headings, split or clipped tables,
unexpected blank pages, crowded margins, and inconsistent lists or spacing.
After a repair, inspect a fresh view or export.

If export is denied or exceeds the API's size limit, retain the completed
structural checks and report the visual limitation. A locally recreated HTML
or Word version is not evidence of how the saved Google Doc renders.

## API references

Consult the relevant reference when constructing that operation:

- [Text and paragraph formatting](https://developers.google.com/workspace/docs/api/how-tos/format-text)
- [Document tabs](https://developers.google.com/workspace/docs/api/how-tos/tabs)
- [Document indices and structure](https://developers.google.com/workspace/docs/api/concepts/structure)
- [Batch ordering](https://developers.google.com/workspace/docs/api/how-tos/batch)
- [Revision control and field masks](https://developers.google.com/workspace/docs/api/how-tos/best-practices)
- [Google Drive export](https://developers.google.com/workspace/drive/api/reference/rest/v3/files/export)
