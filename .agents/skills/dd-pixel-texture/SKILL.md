---
name: dd-pixel-texture
description: Create, edit or improve stylized low-poly pixel-art texture sources for an existing asset with approved UV, including local retouching and wood/metal material variants. Use for texture work without modeling.
---

# Texture request selection
1. Read the user scope: whole asset, chosen material, UV island, mask or component. Do not begin a new mesh creation task.
2. Resolve exact context (TD, atlas size, palette, style, naming) and inspect existing UVs. If the UV target is locked, preserve it.
3. Choose independently among material-first, camera projection and UV-space compositing from `docs/02_UV_AND_PIXEL_PIPELINE.md`. No method is compulsory for every task.
4. Get source art through approved manual images or a provider (future). Generation is optional; never invent successful image output.
5. Treat pixel art as an explicit art-quality gate: work on the target pixel grid with intentional color ramps, readable clusters, controlled edge highlights and material-specific pattern direction. Reject noisy photographic detail and do not claim that nearest filtering, palette remapping or low resolution alone produced pixel art. Follow the cluster and seam guidance in `docs/02_UV_AND_PIXEL_PIPELINE.md`.
6. If the mesh or approved UV is not available, stop at the texture brief: record resolution, TD, palette, material roles and pattern direction, but do not create a pretend final atlas or unwrap implicitly.
7. v0.1 tool `stage_pixel_texture` is ONLY nearest resizing with optional RGB palette remapping to a new PNG; it is not UV-aware and is not finished pixel art. Use it only for those primitives.
8. Once a UV-aware compositor exists, rasterize exact UV/material masks and place layers there, use seam-safe padding and compare on the actual mesh.
9. Preserve locked regions byte-for-byte; stage output, document source hashes and request review before commit. Never overwrite source or approved PNG.
10. Report what was done and what cannot be automated with current tools.
