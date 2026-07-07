---
name: meme-from-template-finite
description: Create memes by compositing images onto classic meme templates using Pillow. Use when the user wants a recognizable meme format with custom elements pasted in — NOT AI-generated.
tags: [meme, image-editing, pillow, composite, template]
triggers:
  - meme template
  - paste onto meme
  - meme format
  - "don't want to play"
  - drake meme
  - distracted boyfriend
---

# Meme From Template

Create memes by downloading a known template image and compositing custom elements onto it with Pillow.

## When to Use

- User asks for a meme using a specific named format/template
- User says "use the actual template" or similar (they do NOT want AI-generated)
- User wants an image pasted over a character in a known meme

## Key Principle

When the user says "make me a meme like X", they usually want the REAL template with edits, not an AI-generated imitation. Only use `image_generate` if they explicitly ask for a generated/artistic version.

## Steps

### 1. Find the Correct Template

Do NOT guess imgflip URLs — they are unreliable and often serve the wrong image or HTML.

**Reliable approach (in order of preference):**

1. **DuckDuckGo image search** — use terminal heredoc (NOT execute_code, which can't see venv packages):
   ```bash
   ~/.hermes/venv/bin/python3 << 'PYEOF'
   from ddgs import DDGS
   with DDGS() as ddgs:
       results = list(ddgs.images("MEME NAME template", max_results=5))
       for r in results:
           print(r['image'])
   PYEOF
   ```
2. **Know Your Meme** — browse via browser tool to find the canonical image
3. **imgflip API** — can identify meme IDs but direct downloads often fail (serves HTML/wrong meme)

**Always verify downloads are real images:**
```bash
# Check magic bytes — JPEG starts with ff d8, PNG with 89 50
head -c 4 /tmp/template.jpg | od -A x -t x1z | head -1
# If you see "3c 21" (<!), "3c 3f" (<?), or "3c 68" (<h) — it's HTML, not an image!
```

- Always verify the downloaded template with `vision_analyze` before editing

### 2. Find the Overlay Image

- Check GitHub APIs for project logos/avatars: `https://api.github.com/orgs/{org}` → `avatar_url`
- For GitHub repos, check README.md for `<img src=...>` tags pointing to logo files
- SVG logos need conversion — but **cairosvg does NOT work** on this runtime (no libcairo.so.2). Use PNG/JPG sources instead, or use `rsvg-convert` if available.
- Organization avatars from GitHub (`avatars.githubusercontent.com`) are good fallbacks for project icons

### 3. Composite with Pillow

```python
# Use the venv python via heredoc (avoids timeout issues with -c flag)
~/.hermes/venv/bin/python3 << 'PYEOF'
from PIL import Image, ImageDraw, ImageFont

# Load template and overlay
template = Image.open("/tmp/template.png").convert("RGBA")
overlay = Image.open("/tmp/overlay.png").convert("RGBA")

w, h = template.size

# Resize overlay proportionally
target_size = int(min(w, h) * 0.35)  # adjust ratio as needed
overlay = overlay.resize((target_size, target_size), Image.LANCZOS)

# Position over the target area (use vision_analyze to determine coordinates)
x_pos = int(w * 0.25)
y_pos = int(h * 0.55)
template.paste(overlay, (x_pos, y_pos), overlay)  # 3rd arg = alpha mask

# Add text if needed
# draw = ImageDraw.Draw(template)
# draw.text((x, y), "TEXT", fill="white", font=font)

output = template.convert("RGB")
output.save("/tmp/meme.png")
PYEOF
```

### 4. Verify and Send

- Use `vision_analyze` on the result to check positioning
- Adjust coordinates and re-run if needed
- Send with `MEDIA:/tmp/meme.png`

## Pitfalls

1. **Wrong template from imgflip**: imgflip's URL scheme (`i.imgflip.com/{id}.png`) often serves HTML pages or unrelated memes. Always check magic bytes AND verify with vision_analyze.
2. **cairosvg broken**: This runtime lacks libcairo.so.2. Don't try to convert SVGs with cairosvg — find PNG/JPG alternatives instead.
3. **Python -c timeouts**: Running `~/.hermes/venv/bin/python3 -c "..."` with complex imports consistently times out. ALWAYS use heredoc (`<< 'PYEOF'`) instead.
4. **execute_code sandbox isolation**: The `execute_code` tool runs in a separate Python environment that CANNOT import packages from `~/.hermes/venv`. For Pillow/ddgs/etc, use `terminal` with heredoc instead.
5. **Pillow not installed**: Install with `~/.hermes/venv/bin/pip install Pillow` first.
6. **Transparency**: Always convert to RGBA before compositing. Convert to RGB before saving as JPEG.
7. **Positioning**: Use `vision_analyze` on the template FIRST to understand where elements need to go, rather than guessing coordinates.
8. **Verify before compositing**: After downloading any image, verify it's a real image file (check magic bytes with `od`). Many meme sites serve HTML redirects or 404 pages that look like successful downloads.
9. **GitHub org avatars**: For project icons, `https://api.github.com/orgs/{name}` → `avatar_url` is reliable and gives a clean PNG. For OpenClaw specifically: `https://avatars.githubusercontent.com/u/252820863?v=4`.

## Dependencies

- Pillow (`~/.hermes/venv/bin/pip install Pillow`)
- curl for downloading templates
- vision_analyze for verifying results
