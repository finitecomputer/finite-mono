---
name: fal-image-editing-finite
description: Edit, inpaint, and outpaint images using fal.ai APIs (FLUX Kontext, FLUX Fill). Covers object replacement, label fixing, outpainting, and reference-guided editing.
triggers:
  - edit an image
  - replace object in photo
  - inpaint or outpaint
  - swap something in an image
  - fix text/label in image
---

# fal.ai Image Editing

## Prerequisites
- fal_client needs `FAL_KEY` env var. The env may have it as `FAL_KEY` or `FAL_API_KEY` depending on context, and terminal subprocesses may not inherit it. Use this robust pattern at the top of every script:
```python
import os
if "FAL_KEY" not in os.environ:
    os.environ["FAL_KEY"] = os.environ.get("FAL_API_KEY", "")
```
If neither variable is available, stop and ask the user to configure the
managed credential. Never print, grep, copy, or hardcode a key from a runtime
file into a command or script.
- Hosted Finite currently requires `fal-client` to be installed for the task;
  use `python -m pip install fal-client` and do not replace Pillow.
- Run scripts with `python`; the hosted runtime places its managed venv first on `PATH`.

## Key APIs

### 1. FLUX Kontext — Instruction-based editing
**Endpoint:** `fal-ai/flux-pro/kontext`
- Best for: whole-object swaps, style changes, simple edits
- Accepts `image_url` (base64 data URI or URL) + text `prompt`
- Supports `image_urls` list for multi-image reference (e.g., pass real product photo as reference)
- Supports `seed` parameter for reproducibility
- Returns 1024x1024 by default
- **Limitation:** Cannot precisely control WHICH part of the image to edit — it may merge/remove nearby objects

```python
result = fal_client.subscribe("fal-ai/flux-pro/kontext", arguments={
    "prompt": "...",
    "image_url": image_data_uri,
    "image_urls": [image_data_uri, reference_image_uri],  # optional multi-image
    "seed": 42,  # optional
})
```

### 2. FLUX Fill — Mask-based inpainting/outpainting
**Endpoint:** `fal-ai/flux-pro/v1/fill`
- Best for: precise regional edits where you need to control exactly what changes
- Requires `image_url` + `mask_url` (white = area to regenerate, black = keep)
- Use Pillow to create masks programmatically
- Set `sync_mode=True`
- Can return base64 data URIs — handle both URL and base64 responses

```python
result = fal_client.subscribe("fal-ai/flux-pro/v1/fill", arguments={
    "image_url": image_data_uri,
    "mask_url": mask_data_uri,
    "prompt": "...",
    "sync_mode": True,
})
```

## Techniques

### Object Replacement (Inpainting)
**Always use FLUX Fill with a mask for object replacement, NOT Kontext.** Kontext cannot control which object it edits and will merge/destroy nearby objects (confirmed repeatedly with overlapping objects like bottle+syringe).

1. Use `vision_analyze` to identify object position — ask for bounding box coordinates on a 1024x1024 (or actual) grid
2. Scale coordinates to actual image dimensions: `scale_x = w / 1024; scale_y = h / 1024`
3. Create a Pillow polygon mask (NOT a rectangle) that follows the object's shape — narrow at narrow parts, wide at wide parts. This gives much cleaner results than a bounding box.
4. Use FLUX Fill with mask + highly descriptive prompt of the replacement object
5. Verify result with `vision_analyze`

```python
# Polygon mask example — better than rectangles
mask = Image.new("L", (w, h), 0)
draw = ImageDraw.Draw(mask)
points = [
    (int(x1 * scale_x), int(y1 * scale_y)),  # narrow top
    (int(x2 * scale_x), int(y2 * scale_y)),  # wide middle
    # ... follow object contour
]
draw.polygon(points, fill=255)
```

### Outpainting (Expanding the frame)
1. Create larger canvas with Pillow, paste original at desired offset
2. Create mask: white for new areas, black for original (add ~10px overlap for blending)
3. Use FLUX Fill with the expanded canvas + mask + prompt describing what should fill the new space
4. Background blends well; hands/fingers at edges often get distorted

### Fixing Text/Labels with Reference Images
1. Get a real product photo (download from brand website)
2. Use Kontext with `image_urls` containing both the edited image AND the reference
3. Prompt: "Make the label in the first image look exactly like the bottle in the second image"
4. This dramatically improves text accuracy over text-only prompts

## Pitfalls
- **AI text rendering:** Diffusion models consistently struggle with correct text on labels. Multiple attempts with different seeds help. Using a real reference image is the best mitigation. Even with a reference, the *word* ("Cholula") will render correctly but other label details (face, sub-text) may still be hallucinated.
- **Kontext merges nearby objects:** When two objects overlap (e.g., bottle + syringe), Kontext often merges or removes one. Use FLUX Fill with a precise mask instead.
- **FLUX Fill may preserve unwanted elements:** If a mask leaves any part of an existing object visible, Fill will sometimes try to blend/preserve it rather than replace it entirely. Make the mask slightly larger than you think you need.
- **Each editing pass degrades:** Hands, fingers, and fine details get worse with each pass. Minimize the number of editing rounds.
- **Handle base64 responses:** FLUX Fill may return base64 data URIs instead of URLs. Always check `url.startswith("data:")`.
- **Coordinate scaling:** When estimating pixel coordinates from vision analysis, vision models often report approximate dimensions that differ from actual. Always load the image with Pillow and use `img.size` for real dimensions — don't trust vision-reported coordinates directly.
- **Try multiple seeds:** Generate 2-3 variants and pick the best one.
- **Ellipse masks for cylindrical/circular displays:** For round lamp cylinders or circular displays, `draw.ellipse([x1, y1, x2, y2], fill=255)` is more appropriate than a polygon and avoids sharp mask edges.

## Workflow for Complex Edits (Proven Two-Pass Pipeline)
This pipeline was validated across multiple edits (product bottle swap, rocket replacement):

1. **Inpaint** the target object with FLUX Fill + polygon mask + descriptive prompt
2. **Outpaint** if needed to show more of the new object (expand canvas, mask new areas)
3. **Fix text/labels** using Kontext with `image_urls` containing [edited image, real product photo from brand website]. This is the KEY step — diffusion models can't render text from description alone but CAN match text from a reference image.
4. Verify each step with `vision_analyze` before proceeding

**Important:** Do NOT try to do everything in one Kontext pass. The two-pass approach (Fill for shape/placement, Kontext for label accuracy) consistently outperforms single-pass attempts.

## When to Use Which API
| Scenario | Use | Why |
|----------|-----|-----|
| Replace object near other objects | FLUX Fill + mask | Kontext will merge/destroy nearby objects |
| Simple style change, whole image | Kontext | No mask needed, instruction-based |
| Fix text/labels on an already-edited image | Kontext + reference image | Reference image dramatically improves text accuracy |
| Expand the frame | FLUX Fill + expanded canvas + mask | Outpainting with mask control |
| Quick one-object swap, nothing nearby | Kontext | Fastest, single pass |
