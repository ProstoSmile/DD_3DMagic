# Current verification, limitations and test plan

## Authored and inspected here

- Source files present: Rust workspace with `dd_core`, `dd_app`, `dd_mcp`; configs and sample PNG; Blender script; Codex Skills.
- Basic algorithms and unit tests **written**: project override precedence, unsafe path rejection, raster undo/alpha preservation, TD on a known triangle, rejection of degenerate triangle.
- `scripts/blender/export_uv.py` is the read-only Blender bridge. `tests/blender_uv_smoke.py` exercises it in a temporary in-memory scene without opening or saving a `.blend` file.
- Sample fixtures `tests/fixtures/square_uv.json` and `demo_project/assets/demo_prop/sources/uv_square_fixture.json` is valid and contains two triangles spanning a 1×1 m plane.

## Initial Phase 1 verification on 2026-11-11

- OS: Windows 11 Home, build 26200. Rust used: `rustc 1.95.0`, `cargo 1.95.0`. The repository requests the moving `stable` channel, but Rustup could not sync it because the toolchain host connection failed; Cargo checks therefore used the installed 1.95.0 toolchain explicitly.
- After Phase 1, `cargo fmt --all`, `cargo check --workspace --all-targets`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings` passed on Rust 1.95.0. Twenty-one `dd_core` tests passed, including the original raster/UV checks and project, migration, naming, unit, hash, and path cases. `Cargo.lock` includes the direct SHA-256 dependency.
- `cargo run -p dd_app` opened the sample project and `source_sample.png`. The UI displayed a 256×256 document and 128 px/m target density. Nearest zoom, palette application, undo, and new-file PNG export were exercised. PNG export produced `demo_project/.assetlab/staging/texture_preview_0001.png` (256×256); the source PNG hash still matches `SHA256SUMS.txt`.
- The Phase 1 UI starts as a native window (`DD_3DMagic | Asset Workstation`). Its startup and selectors call the shared core resolver; core tests and MCP calls verified the demo context. The new project/asset dropdowns were not clicked and the Phase 1 profile was not inspected live in this CLI-only smoke.
- MCP Inspector CLI 2.13.0 listed six tools: `list_projects`, `list_assets`, `resolve_asset_context`, `format_asset_names`, `analyze_uv`, and `stage_pixel_texture`. Workspace discovery returned the demo project and asset; resolved context returned atlas 256×256, target TD 128 px/m, wood/metal material roles, per-property provenance, and matching hashes for the registered source and master reference. Name formatting returned `T_demo_prop_wood_basecolor_256.png` and `M_demo_prop_wood`. A new 128×128 result was staged at `demo_project/.assetlab/staging/phase1_cli_20261111_01.png`; repeating the same request and an output traversal attempt were rejected. `tools/list --strict` reported zero schema errors and 10 portability warnings for nullable `Option` fields.
- Blender 4.5.0 passed `blender --background --factory-startup --python-exit-code 1 --python tests/blender_uv_smoke.py`: schema version 1, 12 cube triangles, 2 plane triangles, 1 m² world area, UV area 1, and 128 px/m for a 128×128 atlas. Non-metric units and overwriting an existing dump were rejected. The smoke test creates geometry in memory and does not touch a project `.blend`.
- With Blender access available, a separate CLI round-trip created a temporary test `.blend`, opened it in a second factory-startup process, and ran `scripts/blender/export_uv.py`: 14 triangles were exported, the 1 m² plane retained world area 1 and UV area 1, and the source `.blend` SHA-256 was identical before and after export. No artist-authored `.blend` was present in the repository, so this validates file loading/export mechanics rather than a production asset.
- A more representative CLI fixture is staged at `demo_project/.assetlab/staging/phase1_crate_uv_fixture_20261111_01.blend` with a PNG preview and verified JSON dump beside it. The crate has 168 non-overlapping UV face islands, three material slots, and 336 exported triangles; all UVs are in 0–1 and measured TD is 128 ± 0.001 px/m on a 2048 atlas. The exporter left the `.blend` SHA-256 unchanged. This is an engineering fixture, not an artist-approved production asset; the current JSON still contains geometry and UV coordinates only, not material-slot mapping.
- The extracted project folder has no `.git` metadata; the archive checksum file was used to confirm the sample source image remained unchanged.

## Reference system verification on 2026-11-11

This section records a later implementation and verification pass. Its counts and results supersede the earlier Phase 1-only Rust test count above.

- `cargo +1.95.0 fmt --all`, `cargo +1.95.0 check --workspace --all-targets`, `cargo +1.95.0 clippy --workspace --all-targets -- -D warnings`, and `cargo +1.95.0 test --workspace` passed. The workspace test result was 57 passed: 12 `dd_bridge`, 44 `dd_core`, and 1 storage integration test.
- The reference editor was exercised in the native window: create a long-shotgun spec; view side, top, front, and three-quarter projections; validate geometry and projections; change barrel depth from 4.92 to 5.00 artistic units; and undo/redo those revisions. The displayed dimension returned to 4.920 on undo and 5.000 on redo. Driving-dimension edit values now persist across egui frames until Apply.
- The typed reference CLI and MCP stdio smoke scripts passed. Coverage included create/apply/undo/redo/render, four views, project path traversal rejection, output collision rejection, Blender creation and saved-file verification, tamper rejection, and checking that verification leaves the published scene unchanged.
- Blender 4.5.0 staged, reopened and checked the long-shotgun proxy. It has 19 components; maximum vertex round-trip error was 2.38e-7, and side/top/front/three-quarter silhouette comparisons passed. This validates the deterministic proxy adapter, not a production-quality firearm mesh.
- The published scene and manifest are at `demo_project/blender/models/props/weapons/long_shotgun/long_shotgun.blend` and `demo_project/assets/long_shotgun/asset.toml`. The immutable r2 reference bundle is at `demo_project/assets/long_shotgun/references/technical/r2_b7e2194c80ea/`; it preserves `geometry.json`, four orthographic outputs, technical SVG, projection/graphic reports, clipped `graphic.png`, original `appearance_input.bin`, and source request metadata. The v1 design brief remains in history; the active manifest references v2.
- The test appearance image was generated separately with Codex ImageGen, then copied into the demo project's staging folder and imported through the typed bridge. DD_3DMagic itself does not call an image generator. The graphic report is `PASS_WITH_WARNINGS`, with semantic appearance `NOT_VERIFIABLE`; contour clipping only checks the exterior silhouette. The image and geometry remain an unapproved art candidate.
- The app now exposes optional project-relative appearance-raster, appearance-notes, and design-brief fields in its collapsed “Optional visual references” group. The selected project's path is the storage root passed to reference operations. The app imports an existing raster and does not generate one.
- `tests/reference_blender_smoke.py`, `tests/reference_mcp_smoke.py`, and the Blender-driven `tests/blender_uv_smoke.py` passed. The UV smoke used Blender 4.5.0 and a temporary directory under D:.
- The extracted workspace has no `.git` metadata, so repository status and a Git diff could not be reported.

## Programmatic silhouette refinement

- Following the art-direction correction, the active test deliverable is a coded Silhouette Reference. Image generation is not used for the silhouette. The older r2 colored graphic remains archived as an unapproved, optional Graphic Reference; the geometry-only publication deactivates it in the manifest without deleting its file.
- The shotgun's primary side contour now uses hand-authored, multi-point extruded profiles for the stock, receiver, receiver plate, fore-end, curved trigger guard, trigger, and sights. The test rejects box primitives in this fixture and checks the profile point counts. Barrel and circular fittings use increased segment counts.
- `reference render` and proxy publication write single-ink `side_silhouette.png`, `top_silhouette.png`, `front_silhouette.png`, and `three_quarter_silhouette.png` at 1440×480. The manifest links its `silhouette_reference` role to the side silhouette. The native panel preview uses the same projection-mask silhouette.
- The silhouette mask is derived from the projection triangles for the same GeometrySpec that drives the other views and Blender round trip. It is not a separately generated image or a hand-authored SVG detached from the geometry source.

## Agent stroke drawing

- The `reference_drawing` MCP tool returned actual PNG canvas frames after each pass of the 400×116 black-on-white shotgun study. The revisions and exported `long_shotgun_line_reference.png` are in `demo_project/.assetlab/staging/shotgun_line_study/`. The agent inspected the outer contour, part boundaries, and detail frames before continuing.
- Capturing the saved detail revision in a new MCP process reproduced the export byte for byte. An out-of-bounds stroke and an attempt to overwrite the export both failed. The MCP smoke covers canvas creation, a stroke, a reopened frame, and rejection of an out-of-bounds point.
- The existing technical bundle `r2_5f7ac220e7be` and its stable Blender scene passed a separate reopen in Blender 4.5.0. The 19 components matched with maximum vertex error of 2.33e-7 artistic units. All four projected silhouettes passed with IoU between 0.99728 and 0.99896. That scene predates the line study. Its agreement with the line study remains open.
- The corrected line study uses a level barrel centerline and a symmetrical taper. The MCP contour and detail frames are `06_level_contour.png` and `07_level_detail.png`. The final drawing for this pass is `long_shotgun_line_reference_v2.png`, with editable strokes in `07_level_detail.json`.
- `requests/prepare_shotgun_line_proxy.py` traces those stroke coordinates into a candidate GeometrySpec. It records the drawing hash and distinguishes estimated transverse dimensions from the traced side contours. The side overlap measured 0.990946 IoU on a 4× registered canvas. The comparison covers the outline and open trigger guard, excluding decorative ink details. See `.assetlab/staging/shotgun_line_proxy_v3/drawing_comparison.json` and `drawing_overlay.png`.
- The candidate scene in `.assetlab/staging/shotgun_line_blender_v3/scene.blend` passed a separate Blender 4.5.0 reopen for all 10 components. Side, top, front and three quarter silhouette IoU ranged from 0.99509 to 0.99902. The staged candidate has estimated widths and has yet to replace the managed scene.
- Rust geometry tests now cover vertical and horizontal double barrel layouts from one document edit, including center offsets and equal measured lengths. The asymmetric fixture independently checks bounds for an inclined nonrectangular profile and unequal cross sections in all three principal views.
- `python tests/reference_drawing_publication_smoke.py` published the corrected reference and proxy to an isolated D: project through MCP. It compared the source, bundled PNG, bundled strokes, geometry and manifest hashes, then removed the isolated input canvas and captured the published strokes byte for byte. The same test independently reopened the stable `blender/models/props/weapons/long_shotgun/long_shotgun.blend` and verified that inspection left its hash unchanged.
- The main demo review packet is `demo_project/.assetlab/staging/shotgun_line_blender_v3_bundled/`. It contains the staged scene, four matched Blender and reference masks, the technical sheet, and the corrected drawing with editable strokes. The separate publish request pins this packet's scene and the matching geometry and stroke inputs. The isolated smoke verifies that a mismatched reviewed hash is rejected and that an accepted publication preserves the reviewed scene's exact bytes.
- An independent read-only proxy review passed against scene SHA-256 `666f61628563e3f0a21f04082e7c50b9d60345ccca65c8411a4ceecca93dd2e0`. The reviewer checked all four matched views, found no mismatches beyond one pixel, and measured a constant barrel centerline at Y = 0 and Z = 0.154 artistic units. All ten mesh components were connected with no loose vertices. The review overlays are in `demo_project/.assetlab/staging/shotgun_line_review_v3/`. This is proxy acceptance evidence. Artist approval and replacement of the demo project's active revision remain pending.
- The demo project passed an MCP preflight against the exact reviewed bundle. `reference_drawing.capture` reproduced the bundled PNG with SHA-256 `7b7d07b7a58be998cb2d08c105fd716141a6b3b34f82e2d6040ae85b40f6f850`. `reference_operation.verify_blender` passed all four views in `demo_project/.assetlab/staging/shotgun_line_api_preflight_d431f60e/comparison.json`. The scene matched the reviewed hash and the active demo scene and manifest matched their preflight hashes after the calls.

## Metric polygon shotgun candidate (2026-11-12)

- Corrected the source-pixel conversion: the former 0.022 value was artistic-relative units per pixel, not meters. At the registered 374 px butt-to-muzzle span, a provisional 1.20 m overall length gives 0.003208556 m/source px. This physical anchor is recorded as provisional and awaits artist confirmation.
- Added `GeometryUnits::Meters` and metric labels, the authored `polygon_mesh` face-loop primitive, closed-manifold/winding/unused-vertex validation, and Blender round-trip checks that preserve face loops. Legacy `triangle_mesh` remains readable. Triangles derived for projection masks are not saved as Blender faces. Blender's read-only report now includes unit system, scale length and display unit, and rejects a meter spec saved with non-metric scene settings.
- Updated the long-shotgun recipe to use the shared side/top station registration and meters per pixel. Three checkpoint scenes were staged through MCP. The revision 9 candidate is in `demo_project/.assetlab/staging/shotgun_lowpoly_metric_v2_03_refined/scene.blend`, SHA-256 `B01B39CA7C9A09B314929929A32D3EB84B93C651D09FB75B4ADF8B9EBECDCEE`. No active scene was replaced.
- Read-only Blender 4.5.0 inspection reports bounds 1.200 × 0.083422 × 0.182888 m, Metric units with `scale_length = 1.0`, 9 components and 308 vertices. Its 272 saved faces contain 262 quads, 10 n-gons and zero triangle faces. Reopen verification passed with maximum vertex error 2.87e-8 m and all three internal render masks passing.
- The separate approved-ink comparison remains 94.44% side and 93.96% top; no 95–100% target was imposed. The enlarged front drawing still has no common scale and remains `NEEDS_EVIDENCE` for exact comparison. Independent review returned `PASS` for the geometry stage only, including scale, polygon topology, semantic parts, registered contours and visible joins. The [staged handoff package](../demo_project/.assetlab/staging/shotgun_metric_v2_handoff/README.md) contains that report; UV, texture and export review remain open.
- The chosen 1.20 m size is a working assumption, not an artist-specified measurement. The graphic's 30 px/m remains a visual target only; UVs, actual TD, textures and export are not done.
- `cargo +1.95.0 fmt --all`, `cargo +1.95.0 check --workspace --all-targets`, `cargo +1.95.0 build -p dd_mcp`, and Python syntax parsing for the changed bridge/model scripts passed. The test suite was not run in this correction pass.

## Still not verified / current limits

### Historical authored low-poly shotgun checkpoint (relative-unit v1)

- The verified scene, drawings, graphic references, and checkpoint evidence are consolidated in [long shotgun handoff v1](../demo_project/assets/long_shotgun/handoff/v1/README.md). The original staged scene was later saved with different component membership; the handoff records both hashes and keeps the verified Blender backup separate from that working edit.
- The MCP modeling workflow built and reopened three separate staged scenes from explicit mesh vertices and triangles. The geometry revision 6 scene is `demo_project/.assetlab/staging/shotgun_lowpoly_03_refined/scene.blend`. It contains 9 separate parts, 308 vertices and 584 triangles. The approved Blender scene was preserved.
- All three actual render PNGs were returned byte for byte by `reference_frame` after each build. The refined scene passed the Blender mesh round trip and the job-mask check in the side, top and front cameras. `model_checkpoints.png` shows the three stages.
- Independent comparisons against the approved ink, using the shared side and top registration, improved from 93.94% and 93.62% to 94.44% and 93.96%. `projection_centerline_review.png` shows residuals. The enlarged front drawing has no approved common scale. A provisional uniform registration using the stock width and muzzle center gave 80.08% and exposed a height/profile difference. Front matching remains `NEEDS_EVIDENCE`.
- Validation used Rust 1.95.0 and Blender 4.5.0. Formatting, workspace checking, strict Clippy, 63 Rust tests, and the stdio MCP smoke passed. The candidate has neutral material IDs/colors and artistic units. UV, actual texel density, and texturing were outside this checkpoint. The artist's 30 px/m override is the target for this asset; the demo project default remains 96 px/m.

- The design targets Blender 5.x, but the available installation is Blender 4.5.0. Blender 5.x compatibility and behavior on a real artist `.blend` remain unverified; the saved-file round-trip used a generated temporary fixture.
- GUI drag-and-drop of an external PNG, the missing-project screen, and click-through of project/asset/material selectors were not exercised in the reference pass. The reference editor itself was exercised live, including its dimension edits and history controls.
- The new native Drawing view builds and passes Clippy. Its runtime interaction remains unverified. The current review uses API and MCP evidence, per the user's instruction.
- Schema v1 manifests migrate in memory only. There is no explicit command yet to write a reviewed v2 manifest; legacy source artifacts remain unpinned until such a migration is approved.
- V2 face/material/reference identifiers are project metadata. They are not mapped to Blender polygon indices or live material slots until the Blender adapter phase.
- No image model is wired into the application. One separately generated test image was imported from staging; no ChatGPT subscription/API key is used or shipped by this repository.
- The imported pixel-art image is a candidate for artist review, not an approved design. Its material/detail semantics are not machine-verified. The proxy geometry is a simple deterministic blockout and should not be mistaken for a finished production model.
- Source art is not automatically mapped onto a UV atlas. Current raster resize/palette operation is a deterministic building block.
- No true texture layer compositor, no UV island segmentation/overlap detector, no editable brush/selection/masks, no dynamic docking and no 3D viewport.
- No auto creation of an external Codex project or GitHub repository. Extract directory and open it as a new standalone workspace in Codex.

## Immediate smoke test commands

```bash
cargo fmt --all
cargo check --workspace --all-targets
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p dd_app
DD_MAGIC_PROJECT_ROOT="$PWD/demo_project" cargo run -p dd_mcp
blender --background --factory-startup --python-exit-code 1 --python tests/blender_uv_smoke.py
```

The MCP server will wait for protocol initialize on stdio; use Codex/MCP Inspector as the client. Raw terminal execution alone isn't a functional test.

## Manual UX test sheet

1. Start editor in root: sample source displayed, no crash. Check correct project profile `demo_prop` displays effective 256×256 and TD 128 px/m from project/style/category/asset.
2. Resize sample from 256 to 64 with nearest; inspect hard pixels in zoom, export to NEW file; verify output is 64×64.
3. Undo operation; preview must be identical to original; source file untouched.
4. Apply project palette, then undo; alpha and exact original recovered.
5. Export same filename twice: must fail instead of replacing previous output.
6. Drag local PNG/JPEG: canvas changes; SVG or huge files produce clear error.
7. Disconnect Blender/MCP: app must remain usable as raster preview editor.
8. Run Inspector and invoke all six tools. Check malicious relative path `../...` rejected, output collisions rejected case-insensitively, and symlink escapes blocked.
9. Blender read-only UV script must not save/mod existing `.blend`, must produce `schema_version=1` JSON with evaluated world coords in meters.
10. Query UV analyzer on 1×1m square full 128×128 atlas: TD ~128 px/m, anisotropy ~1; degenerate triangle noted, not silently counted.

## Other known raster and UV engineering gaps

- `rust-toolchain.toml` selects the moving `stable` channel rather than an exact patch. Rust 1.95.0 passed locally; a fresh stable sync still needs a working toolchain-host connection. `Cargo.lock` is now present for dependency reproducibility.
- PNG staging writer currently uses `create_new` but does not provide a full transactional temp+fsync+commit system. No silent overwrite; a failed encode could leave an incomplete staging artifact. Add cleanup and robust transaction tests.
- Asset exports follow the configured naming pattern and reject collisions. Project-only preview filenames still use a restart-local serial; add explicit version naming if that mode becomes a common artist path.
- Raster `PaletteNearest` measures squared distances in RGB, not perceptual OKLab; use only for preview until validated.
- UV analyzer uses per-face Jacobian and rejects singular mapping. It does not deduplicate overlapping UV faces or verify padding/mip safety.
- Blender exporter requires a `METRIC` scene and still normalizes geometry into meters. Project-side centimeter/inch/feet conversions are covered in `dd_core`, but no Blender adapter currently consumes them.
- Texture images are full RAM buffers; operations run on UI thread in this initial prototype. Add background jobs and size/time limits before large production tasks.

## Completion criterion for the real MVP

On a real chosen low-poly Blender asset (not just test cube): context → UV validity → material region boundaries → UV-accurate material-first paint → editable layers → exact pixel palette and seams → multi-view preview → approve and export to Godot, with no loss of source mesh or approved textures. The current repo gives a documented path to that criterion; it does NOT claim to have satisfied it.
