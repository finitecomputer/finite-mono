---
name: generate-pdf-finite
description: Generate structured, multi-page PDFs from scratch using fpdf2 (simple) or reportlab (rich visuals). Use when the user wants a report, reference document, brochure, or any custom PDF created programmatically. Covers installation, page layout, styling, and common pitfalls. Use reportlab when the user wants high graphical fidelity (dark themes, transparency, custom shapes, glow effects, patterns).
version: 1.1.0
author: community
license: MIT
metadata:
  hermes:
    tags: [PDF, Documents, Generation, Reports, Python]
---

# Generate PDF with fpdf2

Create structured, styled, multi-page PDFs from scratch using Python's fpdf2 library.

## When to Use

- User asks for a PDF document to be created (reports, references, catalogs, etc.)
- Need programmatic PDF generation (not editing an existing PDF — use nano-pdf for that)
- Need full control over layout, styling, headers/footers, and page structure

## Prerequisites

```bash
# Install into Hermes venv
source ~/.hermes/venv/bin/activate
uv pip install fpdf2
```

## Quick Start

```python
from fpdf import FPDF

pdf = FPDF()
pdf.set_auto_page_break(auto=True, margin=25)
pdf.add_page()
pdf.set_font('Helvetica', 'B', 16)
pdf.cell(0, 10, 'My Title', align='C')
pdf.ln(15)
pdf.set_font('Helvetica', '', 11)
pdf.multi_cell(0, 6, 'Body text goes here...')
pdf.output('/home/node/output.pdf')
```

## Key Patterns

### Custom Header/Footer (subclass FPDF)

```python
class MyPDF(FPDF):
    def header(self):
        if self.page_no() > 1:
            self.set_font('Helvetica', 'I', 8)
            self.set_text_color(120, 120, 120)
            self.cell(0, 10, 'Document Title', align='C')
            self.ln(8)

    def footer(self):
        self.set_y(-15)
        self.set_font('Helvetica', 'I', 8)
        self.set_text_color(140, 140, 140)
        self.cell(0, 10, f'Page {self.page_no()}/{{nb}}', align='C')

pdf = MyPDF()
pdf.alias_nb_pages()  # enables {nb} total page count
```

### Styled Sections

```python
# Section title with accent bar
pdf.set_fill_color(47, 54, 64)
pdf.rect(15, pdf.get_y(), 4, 10, 'F')
pdf.set_x(23)
pdf.set_font('Helvetica', 'B', 14)
pdf.cell(0, 10, 'Section Title')

# Info/highlight box
pdf.set_fill_color(235, 245, 255)
pdf.set_draw_color(41, 128, 185)
pdf.rect(15, pdf.get_y(), pdf.w - 30, 12, 'DF')

# Category header with background
pdf.set_fill_color(236, 240, 241)
pdf.cell(0, 8, '    Category Name', fill=True)
```

### Page Break Management

```python
# Check remaining space before adding content
if pdf.get_y() > 250:  # near bottom
    pdf.add_page()
```

## Pitfalls

### CRITICAL: Unicode Characters with Core Fonts

Core fonts (Helvetica, Courier, Times) only support latin-1 characters. **Bullet points (U+2022), em dashes, smart quotes, and other Unicode will crash.**

Bad:
```python
pdf.cell(5, 5, chr(8226))  # CRASHES: UnicodeEncodeError
pdf.cell(0, 5, 'fancy "quotes"')  # May crash with smart quotes
```

Good:
```python
pdf.cell(5, 5, '-')  # Use ASCII dash for bullets
pdf.cell(5, 5, '>')  # Or arrow
pdf.cell(5, 5, '*')  # Or asterisk
```

**To use real Unicode**, add a TTF font:
```python
pdf.add_font('DejaVu', '', '/path/to/DejaVuSans.ttf', uni=True)
pdf.set_font('DejaVu', '', 10)
# Now Unicode works
```

### Available Core Fonts

Only these work without adding TTF files:
- `Helvetica` (B, I, BI)
- `Courier` (B, I, BI)
- `Times` (B, I, BI)
- `Symbol`, `ZapfDingbats`

### Other Gotchas

- `multi_cell()` moves to next line automatically; `cell()` does not
- `set_text_color()` persists until changed — reset after styled sections
- `set_fill_color()` and `set_draw_color()` also persist
- Use `pdf.get_y()` to track vertical position for layout decisions
- `pdf.set_margins(left, top, right)` — call before `add_page()`
- Images: `pdf.image('path.png', x, y, w)` — supports PNG, JPG, GIF

## When to Use reportlab Instead

fpdf2 is great for simple, clean documents. But if the user asks for something "beautiful", "pretty", "high fidelity", "dark themed", or with complex visual design, **use reportlab instead**. It supports:

- **Alpha transparency** on any color (Color objects with alpha parameter)
- **Custom Flowables** for reusable visual components (cards, badges, patterns)
- **Drawing primitives** (circles, polygons, arcs, paths) directly on canvas
- **Multiple page templates** (e.g., special cover page vs normal pages)
- **Platypus layout engine** with Tables, KeepTogether, and automatic pagination
- **Round rects, gradients (simulated), glow effects** via layered transparent shapes

### Install

```bash
source ~/.hermes/venv/bin/activate
uv pip install reportlab
```

### reportlab Quick Start

```python
from reportlab.lib.pagesizes import A4
from reportlab.lib.colors import HexColor, Color
from reportlab.lib.styles import ParagraphStyle
from reportlab.platypus import SimpleDocTemplate, Paragraph, Spacer, Table, TableStyle, PageBreak, Flowable
from reportlab.lib.enums import TA_CENTER, TA_JUSTIFY

W, H = A4

# Page background function
def dark_bg(canvas_obj, doc):
    canvas_obj.saveState()
    canvas_obj.setFillColor(HexColor('#0D1117'))
    canvas_obj.rect(0, 0, W, H, fill=1, stroke=0)
    canvas_obj.restoreState()

doc = SimpleDocTemplate('output.pdf', pagesize=A4,
    leftMargin=28, rightMargin=28, topMargin=36, bottomMargin=36)

# Use onPage callback for backgrounds
from reportlab.platypus.doctemplate import PageTemplate, NextPageTemplate
from reportlab.platypus.frames import Frame

template = PageTemplate(id='dark',
    frames=[Frame(28, 36, W-56, H-72)],
    onPage=dark_bg)
doc.addPageTemplates([template])

story = [Paragraph('Hello', ParagraphStyle('T', fontSize=24, textColor=HexColor('#FFFFFF')))]
doc.build(story)
```

### Custom Flowables Pattern

```python
class AccentBar(Flowable):
    """Colored accent bar for section headers."""
    def __init__(self, width, color, height=3):
        Flowable.__init__(self)
        self.width = width
        self.color = color
        self.height = height

    def draw(self):
        c = self.canv
        c.setFillColor(self.color)
        c.rect(0, 0, self.width, self.height, fill=1, stroke=0)
```

### Key reportlab Patterns

- **Transparency**: `Color(r, g, b, alpha)` where r/g/b are 0-1 floats
- **Glow effect**: Draw multiple transparent circles at increasing radii
- **Cards**: Table with BACKGROUND, BOX, ROUNDEDCORNERS styles
- **Hex/dot patterns**: Custom Flowable drawing in a loop
- **Multiple templates**: Different `onPage` functions for cover vs content pages, switch with `NextPageTemplate('name')` before `PageBreak()`
- **Info boxes**: Table with single cell, styled background, rounded corners

### reportlab Pitfalls

- Colors use 0-1 float range, NOT 0-255 (use HexColor for hex strings)
- `canv.circle(x, y, r)` — center coordinates, not bounding box
- `roundRect` ROUNDEDCORNERS in TableStyle takes list `[tl, tr, bl, br]`
- Flowable `draw()` origin is bottom-left of the allocated space
- `doc.multiBuild(story)` needed if using `doc.page` in onPage callbacks
- Unicode works out of the box with Helvetica/Courier (unlike fpdf2)

## Delivery

After generating, deliver the PDF:
- Telegram: `MEDIA:/path/to/file.pdf` sends as document
- Save to: `/home/node/` for easy access
