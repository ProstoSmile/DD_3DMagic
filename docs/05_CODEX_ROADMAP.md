# Codex Agent — executable implementation plan for DD_3DMagic

**Mission:** deliver an independent, fast, maintainable native Rust artist workbench with controlled Blender/AI operators. Don't force a one-click pipeline or design speculative features. Read `AGENTS.md`, then `00_DESIGN_DOCUMENT_RU.md`, `02_UV_AND_PIXEL_PIPELINE.md`, `03_API_OPERATIONS.md`. Changes must be staged in small verifiable commits.

## Phase 0 — audit supplied scaffold before any feature work

1. `cargo fmt --all`, `cargo check --workspace --all-targets`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` on Windows. This archive was NOT compiled at generation time. Fix API/version mismatches (especially RMCP v3 macros, egui 0.36 methods and `image` encoder). Do not silence compiler warnings by removing important checks.
2. Launch app with `cargo run -p dd_app`, confirm sample loads and nearest zoom works. Check dropped PNG, undo, palette operation, new-file export, missing project error.
3. Start MCP with `DD_MAGIC_PROJECT_ROOT` or `DD_MAGIC_WORKSPACE_ROOT`; verify all six current tools with MCP Inspector. Check clean JSON-RPC, project/asset selection, unsafe paths and duplicate outputs.
4. Run Blender read-only UV extraction against known cube; verify TD for 1 m² plane with atlas 128 and UV mapped 0–1. Add this to scripted smoke tests. Blender API 5.x changes must be pinned to tested version.
5. Write verification report and record versions/OS. If any check fails, fix before adding features.

## Phase 1 — project/asset consistency foundation (implemented 2026-11-11)

Implementation note: workspace/project and asset discovery, v1 in-memory migration, normalized v2 metadata, per-property provenance, source/reference SHA-256 drift reporting, naming and collision checks, unit conversion, and shared UI/MCP context resolution are present. Twenty-one `dd_core` tests passed; MCP Inspector exercised workspace selection and name formatting. New UI selectors launch, but click-through/rendered selection remains part of the manual UX checklist. No migration rewrites existing source or manifests.

Implement explicit project discovery (`workspace` → selected project), naming formatter with collision checks, manifests v2 with migrations, asset UID/face/material identifiers, version stamps, reference roles, immutable source artifact hash, resolved context provenance per property rather than only per file. Add tests for priority, missing profile, malformed value, symlink/path traversal, unit conversion and naming collisions. No automatic category/style inference without evidence.

**Acceptance:** an empty or missing project yields actionable messages. The same asset loaded through UI or MCP resolves identical TD, atlas, material roles and naming. A manual artist change is not wiped by re-discovery.

## Phase 2 — Blender read-only adapter

Create a separate Blender addon/CLI adapter with typed requests, no raw user Python, version handshake, single-thread scheduling with Blender main loop, cancellation/timeouts and host/process state. Inspect mesh topology, material slots, UV layers, seams, evaluated world positions, face-id mapping, object/component locks and direct `.blend` path. Prioritize **read-only** inspection, preview and UV diagnostics. Report exact scene/selection provenance.

**Acceptance:** same known cube/weapon reports correct geometry, UV, TD after modifiers and nonuniform scale. No .blend mutation during inspection. Blender unavailable returns `UNSUPPORTED_PROVIDER`/`BLENDER_OFFLINE` and leaves UI usable.

## Phase 3 — real UV engine / policy

Implement island connectivity, overlaps policy (intentional mirrored/stacked vs error), padding/texel grid validation, polygon UV rasterizer with face/island mask output. Test degenerate UVs, thin islands, triangles across tiling edges and irregular island transforms. Add unwrap & pack through Blender adapter using configurable algorithms; never auto re-unwrap approved UV on a texture request. Capacity estimator with unique islands, useful warnings not fake certainty. Comparative tests for Pixel UV Tools and xatlas are optional research; don't import code without license review.

**Acceptance:** selected mesh to exact 128×128 island masks, stable texel ownership and explicit overlap errors; locked UV never rewritten.

## Phase 4 — actual Photoshop-like document model

Document graph with Source Raster, Raster Layer, Adjustment Layer, Mask, Group, Material Region, Blend Operation and History/Snapshot. Keep transformations non-destructive. Implement dirty regions and CPU cache before GPU rewrite, cancellation, save/reopen of editable `.ddasset` metadata plus referenced images. Tool palettes are task-driven, not dozens of disabled UI replicas.

**Acceptance:** save → close → reopen yields pixel-identical composition; operation stack undo/redo deterministic; target-layer edits don't touch another material; UI remains interactive while processing bounded images.

## Phase 5 — deterministic UV texture compositor

Consume real UV masks+semantic materials; support material-first source and scoped blending on locked regions. Add nearest pixel sampling, perceptual palette quantization (RGB approximation in v0.1 is NOT enough), noise-aware clusters, orientation guides, seam-aware padding and cross-UV boundary checking. Cache texture source hashes and regenerate only impacted regions.

**Acceptance:** wood+steel test asset renders on Blender model without bleed/incorrect rotation; artist-approved metal remains byte-identical after wood retexture; output passes nearest engine preview. All outputs are staged until approval.

## Phase 6 — AI source providers, THEN camera projection

Add provider trait with methods for generating/editing/reviewing art sources; secrets stored in system secure credential store or env var for developer prototypes, never in project file. Prompt includes master reference style/role, not arbitrary UV validity claims. Store approved image and immutable provenance; estimate cost and allow cancellation. Add camera projection only after deterministic compositor: export exact render/visibility/depth, sample generated views into UV with confidence maps and manual hole handling.

**Acceptance:** no external call without user request/approval and disclosed cost; the same approved source can be recomposed into the same UV atlas without model reruns; coverage holes visible. Offline manual-source path always usable.

## Phase 7 — approval and export

Transactions: stage, diff, validate, approve, publish, rollback. Model file hashes and locked states checked before commit. Material assignment in Blender by named regions and Godot import validation. Export GLB+PNG+manifest+import advice only from approved revisions. External artists' `.blend` remains untouched by default.

**Acceptance:** reject leaves approved image/UV/blend unchanged; accepted change can be rolled back; Godot imported material matches Blender preview within expected shader differences.

## Non-goals for the initial agent

Avoid building a full node editor, custom GPU renderer, event bus microservices, scripting language, plugin marketplace, generalized scene engine, rigging, character topology inference, cloud orchestration and elaborate CRT shader before finishing Phases 0–3. Avoid writing an impossible single-call `create_complete_asset` endpoint. Don't create DA dependencies or migrate old DA tools into this repo without explicit user authorization.

## Required final report after each phase

State: implemented file paths, real UI behavior, test commands+output, benchmarks where relevant, unresolved risks, changes to artist workflow, rollback, phase acceptance. No fake screenshots or green checks. Describe UX in artist terms first, code internals second.
