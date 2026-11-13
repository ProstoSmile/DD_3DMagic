---
name: dd-uv-diagnostics
description: Inspect existing Blender UV data, texel density, stretch or atlas capacity without re-unwrapping or changing the model. Use for UV check, UV review, TD requests, or texture preparation.
---

# UV inspection, not automatic unwrap
1. Resolve `asset_id`, actual texture size and TD through project context.
2. Obtain real world-space triangle UV data from Blender. Current bridge is `scripts/blender/export_uv.py`, explicitly run in Blender; it does not execute from the MCP yet.
3. Feed JSON to `analyze_uv`. Verify scene units are meters, nonuniform object scale is reflected, and evaluated modifiers are appropriate.
4. Explain target vs actual TD and the worst anisotropy, which geometry is degenerate and why average TD is insufficient.
5. Check overlapping UVs, island padding and capacity *only when an actual validator exists*. v0.1 does not implement these. State that plainly, never report 'UV valid' based on density alone.
6. Do not modify any UV or apply `uv.unwrap` without a specific user request and review of any existing texture.
7. Tests: 1m world triangle mapped over 128px has 128px/m; flipped triangles should not be falsely considered invalid unless policy forbids them.
