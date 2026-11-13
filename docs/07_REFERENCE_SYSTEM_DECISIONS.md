# Reference system implementation decisions (2026-11-11)

## Phase 0 audit

The extracted workspace has no Git metadata. Preserve existing artifacts; do not relocate it.
`dd_core` owns deterministic context resolution, naming, safe relative paths, raster operations
and UV diagnostics. Reuse these modules. Raster history is an operation stack with undo only;
there is no general command engine, geometry, reference renderer or asset transaction service.
`dd_app/src/main.rs` is the eframe entry point and single editor state, with real raster controls.
`dd_mcp/src/main.rs` is the existing stdio server (six tools); there is no CLI command dispatcher.
`scripts/blender/export_uv.py` is read-only; `build_toz34_asset.py` is a bespoke fixture builder,
not a reusable typed bridge. Tests cover context/path/naming/raster/UV and Blender UV smoke.
Long operations/cancellation and durable multi-file recovery are absent. Existing reference
metadata and authored images remain Design References and are not rewritten.

## Implementation contract

Extend `dd_core` with deterministic geometry/projection/render/command modules. A single
versioned GeometrySpec creates world-space triangulated proxies and every view; no independent
view drawing. Use explicit artistic relative units for the fictional shotgun, never claimed
real-world weapon dimensions. Supported cases receive measured diagnostics; unsupported or
underdefined cases fail explicitly. Keep renderer separate from geometry. Reuse `image` for
direct target-grid pixel illustration; no image model in technical reference creation.

Add managed storage with one explicit D: workspace root, checked relative paths, process-local
environment, new-file staging and journaled recoverable publication under
`blender/models/<category>/<subcategory>/<slug>/<slug>.blend`. Extend current manifest with
optional reference persistence fields rather than replacing v2. Child process launch/Blender
execution belong outside deterministic core. Use one bounded typed Blender script, not arbitrary
code input. Compare reopened actual mesh bounds/component data against Rust-produced expected
geometry before publishing; keep previous files recoverable.

Add reference CLI mode to the existing binary and structured MCP operations, both sharing
the core command contracts with a modest egui reference panel. Do not run Blender on the UI
thread. Preserve raster editing and all old MCP operations. Add no solver/native CAD dependency
unless justified; first solver is bounded and must never advertise universal solving.

## Acceptance and dependencies

Geometry and managed storage can be implemented independently; interface integration follows.
Required checks: fmt/check/test/clippy, negative path/geometry/constraint/dimension tests,
vertical/horizontal cylinder projection changes, deterministic renders, dynamic measurements,
Blender reopen/tamper/save-recovery checks and existing UV/raster regression tests.
Track implemented, unsupported, and not yet verified separately in the final report.

## Solver research and dependency choice

Inspected the actual SolveSpace library wrapper (`src/slvs/lib.cpp`), its typed Rust wrapper
documentation and crate source listing on 2026-11-11. SolveSpace's wrapper uses shared static
solver state and handle-based entities/constraints; the source exposes failure paths and a
bounded native system solve rather than an image-derived geometry contract. `slvs` 0.6.0 is
a native SolveSpace wrapper, so adopting it would add native build and licensing review work.
Keep this first implementation on an explicitly bounded deterministic parameter/relationship
solver. No third-party solver code is copied or linked, and no universal DOF claim is made.

Sources: https://raw.githubusercontent.com/solvespace/solvespace/master/src/slvs/lib.cpp
and https://docs.rs/slvs/latest/slvs/ . The Blender script uses the installed bpy operators
and round-trip tests as executable API evidence; Blender 5.x remains unverified if only 4.5
is installed. `dd_bridge` is a small shared process/workflow crate so neither UI nor MCP
duplicates Blender launching, and `dd_core` retains its ban on subprocess execution.
