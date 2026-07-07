---
name: pdf-workbench-finite
description: Edit and extend PDFs with nano-pdf, then validate the result visually using the local web + Chromium/Playwright loop that works in this Docker runtime.
version: 1.0.0
author: local
license: MIT
metadata:
  hermes:
    tags: [PDF, nano-pdf, Gemini, Playwright, Chromium, review, decks]
    related_skills: [nano-pdf, browser]
---

# PDF Workbench

Use this skill when the user wants to edit or extend an existing PDF, especially presentation decks, proposals, and visual documents.

This runtime has:
- `nano-pdf` installed at `~/.local/bin/nano-pdf`
- local Chromium available through Playwright
- port `3000` forwarded to the host for local review pages

## Required Credential

`nano-pdf` uses Gemini 3 Pro Image and requires:

```bash
GEMINI_API_KEY=...
```

If `GEMINI_API_KEY` is missing, stop and ask for it before attempting edits or adds.

## What nano-pdf does

`nano-pdf` is image-based PDF editing.

For `edit`:
- renders the target page to a high-resolution image
- sends the page image plus your natural-language instruction to Gemini 3 Pro Image
- gets back an edited image
- replaces the original PDF page with that edited image

For `add`:
- uses the PDF plus style reference pages as context
- generates a brand-new page image
- inserts it after the requested page

Important consequence:
- output pages are rasterized image pages in a PDF
- edited text will usually no longer be selectable/searchable like a native vector PDF

## Commands

### Edit existing page(s)

```bash
nano-pdf edit <pdf_path> <page_number> "instruction" [<page_number> "instruction" ...]
```

Examples:

```bash
nano-pdf edit deck.pdf 1 "Change the title to 'Q3 Results' and fix the subtitle typo"
nano-pdf edit deck.pdf 1 "Change the title to 'Q3 Results'" 3 "Make the chart blue instead of red"
```

Useful options:

```bash
--style-refs "5,6"
--use-context
--output custom_name.pdf
--resolution 4K
--disable-google-search
```

Defaults and advice:
- prefer `4K` for important decks
- prefer `--output` so the source file stays untouched
- keep `--use-context` off for `edit` unless the task clearly needs full-document context
- keep Google Search grounding enabled unless you have a reason to disable it

### Add a new page

```bash
nano-pdf add <pdf_path> <after_page> "description of new page"
```

Examples:

```bash
nano-pdf add deck.pdf 0 "Create a title slide for Q3 Review with a clean investor-deck style"
nano-pdf add deck.pdf 7 "Add a closing slide with key takeaways and next steps"
```

Useful options:

```bash
--style-refs "1,2"
--no-use-context
--output custom_name.pdf
--resolution 4K
--disable-google-search
```

Defaults and advice:
- `add` uses full PDF text context by default, which is usually desirable
- use style references aggressively for decks so the new page matches the existing visual language

## Recommended Workflow

1. Clarify exactly which page(s) should change and what must remain untouched.
2. Work inside a dedicated job folder under `/home/node/workspace`.
3. Keep the original PDF unchanged. Always write to an explicit output path.
4. Prefer `4K` resolution unless speed matters more than fidelity.
5. If matching an existing deck, pass `--style-refs`.
6. After generation, visually validate the output before claiming success.

## Validation Workflow That Works In This Runtime

Do not rely only on file existence. Visually inspect the rendered output.

Direct Chromium navigation to a raw `.pdf` URL in this runtime may trigger a download instead of rendering, so use a tiny HTML wrapper page that embeds the PDF.

### Step 1: Start a local review server from the directory containing the PDF

```bash
cd /home/node/workspace/pdf-job
python -m http.server 3000
```

### Step 2: Create a lightweight viewer page

```html
<!doctype html>
<html>
  <head>
    <meta charset="utf-8">
    <title>PDF Review</title>
    <style>
      html, body { margin: 0; padding: 0; background: #111; }
      iframe { width: 100vw; height: 100vh; border: 0; }
    </style>
  </head>
  <body>
    <iframe src="/edited_deck.pdf#page=1"></iframe>
  </body>
</html>
```

Save it as `pdf-viewer.html` in the same directory.

### Step 3: Capture a screenshot with Playwright

```bash
npx playwright screenshot --browser=chromium   http://127.0.0.1:3000/pdf-viewer.html   review-page-1.png
```

### Step 4: Review multiple pages

Change the iframe URL fragment and repeat:

```html
<iframe src="/edited_deck.pdf#page=3"></iframe>
```

Then capture another screenshot.

### Step 5: Compare before/after when needed

Serve both the original and edited PDFs from the same folder and make separate viewer pages for each, or swap the iframe target and capture screenshots for the same page number from both files.

## What To Validate

Check these visually:
- the requested text/content change actually happened
- unchanged elements still look consistent
- typography is legible
- charts/tables did not get mangled
- brand colors and layout still make sense
- the right page was edited
- the page count/order is still correct after `add`

## Common Gotchas

- page numbers can behave like 0-based or 1-based depending on context; if the wrong page changed, retry with ±1
- small text and dense layouts are much more reliable at `4K`
- `--use-context` can confuse `edit` jobs
- output is rasterized, not structurally editable PDF content
- the model can take creative liberties; always validate visually
- generation can take 15-30 seconds per page at `4K`

## Relationship To Other Tools

- use web/browser tools first if the user needs reference material, brand assets, logos, or current factual grounding
- use `nano-pdf` for the actual PDF edit/add operation
- use the local web + Playwright review loop above to validate the output before delivery
