# DD_3DMagic: permanent project guidance for Codex

This repository is an independent Rust asset-production tool, NOT a Godot project, Photoshop clone, or feature inside DA. The human is a mid-level generalist character/3D artist and retains artistic control. Keep behavior modular: operations, policies, recipes, and renderers are separate concepts.

## Start
1. Read `docs/00_DESIGN_DOCUMENT_RU.md` and `docs/06_VERIFICATION_AND_LIMITS.md` before modifying the architecture.
2. Use `docs/05_CODEX_ROADMAP.md` for sequencing and acceptance gates.
3. Inspect actual files and tests before proposing changes. Report implemented, unimplemented, unverified separately.
4. For image/UV/pixel operations, consult `docs/02_UV_AND_PIXEL_PIPELINE.md` before touching code.

## Architectural invariants
- A request like "make a texture" must NOT trigger mandatory model creation, unwrap or export. Compose only needed tools.
- `dd_core` is deterministic: no editor/UI dependencies, HTTP, Blender execution or model calls.
- `dd_app` presents editor state and invokes domain commands; it owns no art pipeline logic.
- `dd_mcp` exposes structured tools over local stdio. No arbitrary Python/code execution tool.
- Blender is external. Never turn this repo into a `.blend`-centred project or embed Godot.
- `ProjectContext` resolution is explicit: project < style < category < asset < user override. Preserve provenance.
- Never modify locked geometry, UVs or texture regions implicitly. Source remains immutable; stage results before commit.
- The CRT visual effect is **UI-only**. It must never alter exported texture pixels or displayed color-critical art when pixel inspection is requested.
- Never claim that naive nearest-downscaling creates art-quality pixel art.
- No invented features: do not pretend mock panels, unfinished MCP tools, network image calls or viewport work.

## Security and preservation
- All MCP file inputs must be project-relative and path checked; all MCP outputs must be new and under staging unless separately approved.
- No arbitrary Blender `exec` forwarded through general MCP tools. Prefer bounded, typed operations and read-only scripts.
- Avoid license copying; use open-source projects for analysis and references only unless license obligations are reviewed.
- Keep operation logs, inputs and outputs explainable. Establish rollback before introducing destructive edits.

## Commands
```
cargo fmt --all
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```
If Cargo or Blender is unavailable, say so explicitly. Do not mark checks as passed.

## Working with the artist
- Follow the staged reference-to-model sequence in `docs/08_ASSET_WORKFLOW.md`.
- Author new model dimensions and coordinates in meters, with Blender Metric units and `scale_length = 1.0`; convert registered reference pixels through one explicit real-world dimension.
- Keep artist-editable mesh faces as quads and useful n-gons. Do not triangulate saved geometry; triangle meshes remain a legacy input format, and renderer-only derived triangles must not replace the Blender face loops.
- For a new model or an agreed geometry revision, use `$dd-master-reference` to separate the design brief from the versioned technical sheet. Use front, side, and top orthographic views. Carry that version into Blender and compare the three matched views at acceptance.
- At a model handoff or agreed acceptance milestone, use `$dd-model-review` for an independent technical and visual verdict before advancing the asset. Skip this gate for unrelated operations and work in progress.
- Describe the exact selected asset, targeted regions, chosen TD and palette before mutation.
- Do not silently resize atlases, repack UVs or overwrite edited artist content.
- Report the changes, technical diagnostics, visual issues and rollback path in human terms.
- Small useful increments over speculative platform architecture. Each stage needs tests and visible output.
