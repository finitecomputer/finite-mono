---
name: layout-check-finite
description: Format and verify Google Docs created or edited through the Google connection, and check website layout before final delivery. Use for headings, lists, spacing, tables, clipped or overlapping text, and mobile layout defects.
---

# Layout Check

Make the saved output readable in the surface the user will open. Apply this
check during creation or repair, and before calling the result finished.
Preserve the user's content, template, and chosen design.

## 1. Choose the surface

- **Google Docs:** read `references/google-docs.md` before writing or formatting.
  Use `google-workspace-finite` for the existing Google connection.
- **Websites:** use `website-building-finite` for the build and browser setup;
  apply the website checks below to simple static pages as well as apps.

Use the specific document or project identified by the task. Ask for a target
only when it is ambiguous. This skill supports the requested edits; it does not
authorize changing sharing settings or publishing additional outputs.

## 2. Inspect the saved result

Check the latest saved version after fonts and images have loaded. Inspect the
rendered output with an available image or browser inspection tool. Creating a
screenshot, reading extracted text, or receiving a successful API response alone
does not establish visual correctness.

For a new draft, make a brief check of the main view and reveal it promptly.
Before final delivery, inspect the changed content and surrounding layout:

- Text is readable and complete, with no clipping, overlap, missing glyphs,
  or accidental formatting markers.
- Headings, paragraphs, lists, and tables have consistent spacing and alignment.
- Images retain their proportions and fit their containers.
- Long headings, links, and table cells wrap without hiding content.

For **websites**, inspect desktop and mobile views, scroll through the changed
page, and check any affected navigation or expanded state. Catch unintended
horizontal page scrolling. Keep wide data tables usable with local scrolling
when appropriate. Use wrapping and responsive sizing to fix overflow; preserve
content and readable type. Inspect the served version as well when deployment
is part of the user's task.

For **Google Docs**, verify the saved structure and use the live document or a
Google-rendered export as described in the reference. Check page boundaries
when the document uses pages.

## 3. Repair and verify

Fix observed defects in the source, then inspect a fresh rendering of the
affected area and its neighbors. Content movement can affect later pages or
sections. Stop when the requested area is readable and the defect is resolved;
a clean inspection needs no cosmetic changes.

After two unsuccessful corrections to the same defect, explain what remains
and what would resolve it. Keep unrelated design changes outside this pass.
If rendering or image inspection is unavailable, complete the structural
checks you can perform and state that visual layout remains unverified.

## Completion

Return the document or website link with a brief account of what was checked,
what was corrected, and any remaining limitation. Distinguish a checked sample
from a full-document review. Claim visual verification only for output actually
inspected after the final edit.
