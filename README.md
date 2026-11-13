# DD_3DMagic

**Standalone Rust-first asset workstation + local MCP tools for an artist-driven Blender workflow.**  
**Status: foundation / v0.1 scaffold. Not a finished 3D texturing product.**

Start here: **[docs/00_DESIGN_DOCUMENT_RU.md](docs/00_DESIGN_DOCUMENT_RU.md)**.

## What is already in the repository

| Component | Actually included | Not yet implemented |
|---|---|---|
| Native editor | CRT-inspired workbench; workspace/project and asset selectors; material-surface selection; PNG/JPEG input; nearest-neighbor preview; non-destructive raster operations; profile provenance; project naming on new PNG output | True paint/layer compositing, draggable docks, 3D viewport, brushes, UV canvas, masks |
| Rust core | Workspace discovery, project/asset selection, read-only v1→v2 manifest migration, layered project/style/category/asset resolution, per-property provenance, face/material/reference IDs, source-hash drift reporting, unit conversion, safe naming/collision checks, raster operations, UV density analysis | UV unwrap/pack/overlap detection, GPU baking, island masks, projection, semantic surface recognition |
| Local MCP | Rust stdio server with six tools: `list_projects`, `list_assets`, `resolve_asset_context`, `format_asset_names`, `analyze_uv`, `stage_pixel_texture` | Blender interaction through a typed bridge, image-generation service, transaction approval workflow |
| Blender integration | A read-only inspection **script** exporting evaluated world-space triangles + UV; requires Blender to run manually | Running the script from MCP, selecting active Blender objects interactively, mutation-safe adapter |
| Codex | `AGENTS.md`, repo-local skills, implementation sequence | Automatically created Codex UI project or Github repository |

**Archive note:** the original archive was authored without Rust/Cargo and Blender execution. Current local verification results, tested versions, and remaining gaps are recorded in [docs/06_VERIFICATION_AND_LIMITS.md](docs/06_VERIFICATION_AND_LIMITS.md).

## Quick start on Windows

1. Install current stable Rust with `rustup` and install Blender 5.x separately. Local Blender 4.5.0 exporter smoke passed; 5.x remains unverified.
2. Extract the archive; open **the `DD_3DMagic` folder itself** in Codex or your IDE. This is a separate repo/workspace, independent of Godot and DA.
3. At the repository root:

```powershell
cargo check --workspace --all-targets
cargo test --workspace
cargo run -p dd_app
```

Run from a workspace directory: the app discovers child folders with `.assetlab/project.toml`; choose the project and asset in the left panel. If exactly one project and asset are found, they are preselected. `DD_MAGIC_WORKSPACE_ROOT` and optional `DD_MAGIC_PROJECT_ID` select a workspace/project explicitly; `DD_MAGIC_PROJECT_ROOT` keeps direct project-root launch compatibility. Output goes to the selected project's `.assetlab/staging/`, never automatically into an engine project.

Schema v1 project and asset manifests migrate in memory to the normalized v2 model. Discovery never rewrites manifests or source art. V2 source artifacts require pinned SHA-256 values; changed or missing inputs are reported in the resolved context.

## Run the local MCP

Build first:

```powershell
cargo build -p dd_mcp --release
```

Register in Codex (adjust absolute paths, PowerShell illustration):

```powershell
codex mcp add dd_3dmagic --env DD_MAGIC_PROJECT_ROOT="D:\\YourProjects\\DD_3DMagic\\demo_project" -- "D:\\YourProjects\\DD_3DMagic\\target\\release\\dd-3dmagic-mcp.exe"
```

Use `codex mcp list` to confirm registration. Alternatively set `DD_MAGIC_WORKSPACE_ROOT`, call `list_projects`, then pass the selected `project_id` to `list_assets`, context, naming, UV, and staging tools. **Do not install/run the app and MCP as one inseparable process.** The UI and agent can operate independently against the same project files; parallel write coordination is still a planned requirement.

For local Blender UV extraction:

```powershell
blender -b "path\to\your.blend" --python scripts/blender/export_uv.py -- --out "absolute\path\to\your_project\.assetlab\staging\uv_dump.json"
```

A manually exported JSON can be passed into `analyze_uv` through the MCP tool. The script deliberately does not modify the .blend file.

Run the scripted exporter smoke test on a temporary in-memory cube and plane:

```powershell
blender --background --factory-startup --python-exit-code 1 --python tests/blender_uv_smoke.py
```

## Documents

- `docs/00_DESIGN_DOCUMENT_RU.md` — authoritative product and architecture document.
- `docs/01_RESEARCH_AND_SOURCES.md` — documented source analysis and technology choices.
- `docs/02_UV_AND_PIXEL_PIPELINE.md` — detailed UV/baking/pixel-art research and algorithms.
- `docs/03_API_OPERATIONS.md` — contracts, state machine, protection and tool inventory.
- `docs/04_UI_DESIGN_CRT.md` — Photoshop-inspired editor logic and CRT design tokens.
- `docs/05_CODEX_ROADMAP.md` — actionable sequential implementation brief for Codex.
- `docs/06_VERIFICATION_AND_LIMITS.md` — acceptance criteria and current limitations.
- `schemas/` — UV export v1 JSON Schema and **future** operation envelope (not yet exposed by MCP).

## Boundary

DD_3DMagic is **not** part of Desire Archive, not a Godot project, and not a replacement for Blender. The program aims to inspect and refine inputs, manage project conventions, generate artistic sources via pluggable providers (future), and guide/review Blender operations with a reproducible workflow. It must remain useful without any AI API key.
