---
name: fal-image-editing-finite
description: Generate, edit, transform, and reference-guide images through Hermes's managed image tool, backed by Finite's configured FAL model.
triggers:
  - generate an image
  - edit an image
  - replace object in photo
  - transform an image
  - fix text or label in image
---

# Managed FAL Image Generation and Editing

Use Hermes's native `image_generate` tool for every FAL-backed image request.
The native tool owns the managed credential and provider call. Never try to
read `FAL_KEY`, install or import `fal_client`, or call a FAL endpoint from
terminal/Python code.

## Supported native contract

Call `image_generate` with:

- `prompt`: required, detailed text describing the image or requested edit.
- `aspect_ratio`: optional `square`, `landscape`, or `portrait`.
- `image_url`: optional public URL or absolute conversation-local path for the
  source image to edit.
- `reference_image_urls`: optional list of additional source/style/composition
  image URLs or absolute paths.

Omit image inputs for text-to-image. Pass `image_url` for an edit. Add
`reference_image_urls` when the model should preserve a product, character,
style, or composition from other images.

The tool description reports the active model's current capabilities. If it
says the model is text-only, do not pass image inputs. If an edit call reports
that the active model cannot edit, explain that limitation and stop; do not
fall back to a direct provider call.

## Editing workflow

1. Inspect the source and identify exactly what should change and what must
   remain unchanged. Use `vision_analyze` when spatial or visual details are
   uncertain.
2. Write a focused edit prompt that names the target, replacement, preserved
   elements, lighting, perspective, and desired finish.
3. Call `image_generate` with the source in `image_url`.
4. For product identity, character consistency, label appearance, or style
   matching, add the best reference images in `reference_image_urls`.
5. Inspect the result and make the smallest useful follow-up edit. Repeated
   full-image passes degrade hands, fine detail, and text.

## Prompting guidance

- State both the requested change and the invariants: “Replace the red mug
  with the reference bottle; preserve the person's hand, table, framing,
  shadows, and background.”
- For text and labels, provide a clean reference image whenever possible.
  Image models may still misspell small text, so verify it rather than claiming
  exact typography.
- For nearby or overlapping objects, describe the target's location and the
  neighboring objects that must remain untouched.
- Prefer one precise edit over a broad restyling request when fidelity matters.

## Honest limitations

The native Hermes tool does not currently expose a mask parameter. Do not
claim pixel-mask inpainting or direct FLUX Fill support, and do not bypass the
native tool to obtain it. If the request requires a hard mask boundary, explain
that the managed editing contract cannot guarantee it and ask whether a
best-effort unmasked edit is acceptable.

Outpainting can be attempted only as a normal native image edit: prepare an
expanded source canvas if useful, pass it as `image_url`, and describe how the
new area should continue. Do not present this as mask-controlled outpainting.

## Result handling

The tool returns the result in `image`, as a URL or absolute path. A successful
local result may also include `agent_visible_image` for later tool operations.
Use the platform's normal file-delivery convention to show the result, and
never expose provider credentials or internal credential paths.
