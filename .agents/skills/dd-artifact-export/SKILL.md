---
name: dd-artifact-export
description: Prepare approved staged 3D assets, PNG atlases and material settings for game engine export with naming, texture filter and rollback checks. Use for publish/export requests.
---

# Export gates
1. Only operate on an approved asset revision. Never interpret a staged texture as publish-ready.
2. Validate asset ID, UV hash, target engine, paths, material channel assignment and naming convention.
3. Keep original `.blend` and artist-approved textures unchanged; export creates versioned output.
4. For Godot, verify per-material texture filter, mipmaps and import compression on an engine smoke test. The Godot importer may enable 3D VRAM compression; this can damage pixel art.
5. Report GLB compatibility, external texture paths, scene orientation/scale, material check and evidence.
6. v0.1 has only staging PNG creation, not a Godot exporter or commit engine. State this openly and direct development to Phases 7 of roadmap.
