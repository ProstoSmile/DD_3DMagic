---
name: dd-project-context
description: Resolve exact DD_3DMagic project style, asset naming, texture density and editable scope before any modeling/texturing request. Use for requests about project conventions or unexplained values.
---

# Project context first
1. Call local MCP `resolve_asset_context` with the actual asset_id if present; if the MCP isn't connected, read `.assetlab/project.toml`, style and category patches, and the specific `assets/<id>/asset.toml` using the documented precedence.
2. Report effective `width/height`, target TD, tolerance, padding, palette, style, and exact provenance. Do not choose missing export destination or image-provider credentials by guesswork.
3. Inspect `protected_components` and manual artwork before mutation. A written request to change color is not an unlock command.
4. Validate proposed name against `naming` templates and ensure it does not collide with existing approved output.
5. If source artifacts refer to a different UV map/revision, stop and surface the mismatch.
6. Tool status: `resolve_asset_context` implemented in v0.1; detailed semantic locks, per-field provenance and naming formatter are not yet implemented. Do not claim they are.
