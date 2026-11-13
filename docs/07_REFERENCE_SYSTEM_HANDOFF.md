# DD_3DMagic reference system handoff

## Architecture audit

The starting project had the Rust workspace, native egui editor, stdio MCP server, asset and project resolution, raster operations, and a read-only Blender UV script. The implementation keeps these entry points. `dd_core` owns deterministic geometry, projection, dimension measurement, validation, and managed storage. `dd_bridge` handles shared reference operations, drawing capture, and bounded Blender jobs. The editor, CLI, and MCP call these owners.

## Implementation summary

- Versioned GeometrySpec and reference history support component edits, constraints, driving dimensions, and undo/redo. Front, side, top, and three-quarter projections come from one proxy. SVG labels derive their values and units from measured geometry.
- The MCP drawing canvas records black strokes on white and returns a PNG after each pass. The corrected shotgun drawing is the design source. Its side contour was traced into GeometrySpec, with transverse dimensions specified as artistic estimates.
- Blender builds or reopens a proxy and reports component and silhouette agreement. Managed publication can preserve the exact reviewed scene and store both the editable drawing and its rendered PNG.

## Data and file layout

The project root is `D:/GitHub_rep/DD_3DMagic/demo_project`. Staging, backups, transaction journals, and configurable Blender child data live under `.assetlab`. Published references live under `assets/<slug>/references/technical/` and the asset manifest is `assets/<slug>/asset.toml`. The active scene path is `blender/models/props/weapons/long_shotgun/long_shotgun.blend`.

The revision 3 candidate is in `.assetlab/staging/shotgun_line_blender_v3_bundled/`. Its `reference/` directory contains the black line drawing, stroke source, geometry, and technical sheet. The independent overlays are in `.assetlab/staging/shotgun_line_review_v3/`.

## Validation evidence

| Requirement | Evidence |
| --- | --- |
| Vertical, horizontal, and asymmetric fixtures | `double_barrel_reorients_all_principal_views_from_one_document` and `inclined_asymmetric_profile_has_expected_ortho_bounds` assert centers, dimensions, and orthographic bounds. |
| Dimensions and diagnostic failures | `driving_dimensions_apply_atomically_and_solve_stored_values`, `invalid_driving_and_relation_anchors_have_error_diagnostics`, `conflicting_driving_dimensions_fail_without_mutating_the_document`, and `unsupported_or_underdetermined_geometry_cannot_be_exported`. |
| Orthography and deterministic output | `rejects_reflection_stale_revision_and_fabricated_projection_coordinates` and `renderers_are_deterministic_and_sheet_marks_convention_and_dimensions`. |
| D: storage and recovery | `root_is_d_only_and_relative_paths_reject_traversal`, `checked_paths_reject_junction_escapes`, and `simulated_interruption_is_recovered_from_journal`, plus the storage baseline integration test. |
| Exact staged candidate | MCP reopened the drawing and scene in the demo project. `.assetlab/staging/shotgun_line_api_preflight_d431f60e/comparison.json` records Blender 4.5.0, 10 components, a maximum vertex error of 2.33e-7 artistic units, and four silhouette IoUs of 0.99509–0.99902. Scene, manifest, and drawing hashes were checked. |
| Managed publication and independent tamper detection | `python tests/reference_drawing_publication_smoke.py` published the exact reviewed bytes and reopened the bundled drawing after removing the scratch input. `python tests/reference_blender_smoke.py` rejected a modified scene and verified the active fixture scene hash. Both used isolated D: projects. |

## Known limitations

The constraint solver covers explicit supported parameters and relations. Unsupported geometry is diagnosed and blocked from technical export. Proxy widths are artistic estimates. The independent review covers the technical proxy and silhouette only. Blender 5.x and the new Drawing tab’s interactive behavior are unverified. Current verification uses API and MCP.

## Regression results

Rust formatting, workspace checking, strict Clippy, and the 62-test workspace run passed in the reference implementation. The latest dimension-label change passed strict Clippy and its focused Rust regression. The MCP smoke passed with checks for `5.500 au` and `-10.000 °` in the exported sheet and a 5.5-unit barrel span in both the side and top projections. It also covers the existing project and asset context, create, edit, undo, redo, export, path rejection, and drawing capture. Historical results and version details are recorded in `06_VERIFICATION_AND_LIMITS.md`.

## Approved publication

The artist approved the corrected shotgun. MCP published the reviewed revision 3 using `demo_project/requests/publish_shotgun_line_proxy_v3.json`. The active manifest points to `assets/long_shotgun/references/technical/r3_8f122803b18b_d4828858fa912/`. The stable scene matched the reviewed SHA-256 byte for byte and passed a new four-view reopen. The previous scene and manifest are in `.assetlab/backups/42000-1794424487680300300-1/`. The verification report and black line four-view preview are in `.assetlab/staging/shotgun_line_published_verify_v3/`. The preview pairs the approved side drawing with top, front, and three-quarter outlines from the published projection data.
