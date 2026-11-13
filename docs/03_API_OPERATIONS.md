# Tool / Operation Contracts — interface design

## 1. Терминология

**Operation** — одна проверяемая трансформация или запрос: `uv.analyze`, `texture.quantize`, `asset.resolve`. **Recipe** — декларативный DAG из операций с условиями и явными входами, выбираемый под запрос; workflow не равен hardcoded pipeline. **Skill** — инструкция Codex, когда и зачем вызывать операции. **Provider** — внешняя реализация (Blender, image model, file), поддерживающая объявленные capabilities. **Adapter** — программный фасад над provider.

## 2. Реализованные MCP capabilities (Phase 1)

- `list_projects({})` discovers configured project roots in the bound workspace. A project root can also be passed directly through `DD_MAGIC_PROJECT_ROOT`.
- `list_assets({"project_id":"demo_stylized_fps"})` lists validated asset manifests from an explicitly selected project when the server is bound to a workspace.
- `resolve_asset_context({"project_id":"demo_stylized_fps","asset_id":"demo_prop"})` reads project, style, category, and asset rules. A direct project binding needs only `asset_id`; `asset_id:null` resolves project/style context. The result includes effective TD/atlas/naming, material and face IDs, reference roles, hash states, and per-property provenance. Read-only.
- `format_asset_names({"project_id":"demo_stylized_fps","asset_id":"demo_prop","surface":"wood","channel":"basecolor"})` formats texture and material names from the selected project's patterns and reports whether the texture name is available in staging. Read-only.
- `analyze_uv({"project_id":"demo_stylized_fps","asset_id":"demo_prop","uv_dump_relative":"assets/demo_prop/sources/uv_square_fixture.json"})` analyzes world-space UV data. In a direct project binding omit `project_id`. Resolution and target TD use the same core context as the editor. The report includes per-triangle metrics and TD/anisotropy warnings. It does **not** detect UV overlaps, seams, islands, or material-region masks.
- `stage_pixel_texture({"project_id":"demo_stylized_fps","asset_id":"demo_prop","source_relative":"assets/demo_prop/textures/source_sample.png","output_relative":".assetlab/staging/preview_0001.png","width":128,"height":128,"use_project_palette":true})` reads a project image, applies deterministic nearest resize and optional palette snap, and writes a new PNG directly in staging. It does not connect Blender/AI or claim artistic texture generation. Path traversal and symlink escapes are rejected; existing names are rejected case-insensitively.

Set `DD_MAGIC_PROJECT_ROOT` to bind directly to one project, or set `DD_MAGIC_WORKSPACE_ROOT` and call `list_projects` before selecting a `project_id`. `DD_MAGIC_PROJECT_ID` can preselect one workspace project at process startup. Stdio is launched separately by Codex; the UI and agent use the same `dd_core::resolve_context` implementation. Skills remain orchestration instructions, while MCP exposes these bounded capabilities. Error results use `ok:false,error`.

## 2.1. Technical reference operations (implemented)

The existing `reference_operation` MCP tool and the `dd-3dmagic reference <request.json>` subcommand share the typed `dd_bridge::ReferenceRequest` contract. The request file and every input path are project-relative; outputs must be fresh subdirectories of `.assetlab/staging` on D:. The command reads no arbitrary code. Set `DD_MAGIC_PROJECT_ROOT` for a direct project, or run it with the project as the current directory.

### Agent drawing canvas

Call the MCP tool `reference_drawing` with a typed `request`. Use `create` with `width`, `height` and `output_relative` for a blank canvas. Use `stroke` with `source_relative`, a fresh `output_relative`, and `strokes`. Each stroke contains a brush `radius` (0 through 4 pixels) and an ordered list of `points` as integer `[x, y]` pairs from the top left. The rasterizer draws a continuous black line through those points on white. Each successful call returns metadata and a PNG image directly in the MCP response. Inspect that frame before deciding where to draw next.

`capture` reopens a saved revision and returns its PNG without writing. `export` writes a fresh PNG via `output_relative`. Use `.json` for revisions and `.png` for exports. All new files go under project-relative `.assetlab/staging` on D:. To revise a pass, branch from the preceding JSON. A canvas records the design drawing in pixel coordinates. The technical views and Blender proxy still derive from a separate GeometrySpec with depth and cross sections.

To include a drawing in a Blender reference bundle, pass `drawing_reference_relative` on the shared `blender` request. Use it in place of `design_reference_relative`. The bridge saves a byte-for-byte copy of the editable stroke file, renders its PNG, and records both hashes and the geometry hash. With `publish: true`, the manifest pins the bundled PNG and stroke JSON under the immutable technical revision. The drawing remains editable through `capture` and by branching to a new staged canvas.

### Authored model checkpoints

For new artist-editable solids use `primitive.kind: "polygon_mesh"`, `parameters.vertices` as `{x,y,z}` coordinates, and `parameters.faces` as zero-based loops of four or more vertex indices. The core preserves these face loops and rejects triangle faces, invalid indices, degenerate/self-intersecting faces, inconsistent edge winding, open edges, disconnected shells and unused vertices. One node represents one semantic component. `triangle_mesh` remains readable for legacy GeometrySpecs. Projection masks may use derived triangles internally; the Blender scene is built from the original polygon loops and round-trip validation checks those loops. New physical model specs use `units: "meters"`; Blender scenes set Metric units and `scale_length = 1.0`. Convert reference pixels using an explicit metric anchor before authoring coordinates. With `technical_sheet_layout: "orthographic_three"`, export and Blender verification use only side, top, and front. Older sheets retain their declared layout.

Call `reference_operation.blender` with `publish: false` for each staged checkpoint. Use `reference_frame` with `stage_relative` and `view` set to `side`, `top`, or `front` to retrieve the saved Blender render as an inline PNG. Compare the rendered shape against the independently registered silhouette between stages. Blender's built-in comparison against its job geometry establishes round trip fidelity, which is separate from agreement with the artist's drawing.

For publication after review, set `publish: true` and supply `reviewed_proxy` with `scene_relative` (a staged `.blend`), `scene_sha256`, `geometry_sha256`, and `drawing_sha256`. This path uses the editable drawing input. The bridge checks all three hashes, copies the exact staged scene, reopens the copy with Blender, and runs the same publication transaction as a fresh build. A mismatch stops before the transaction.

Supported operations are `create`, `inspect`, `apply`, `undo`, `redo`, `render`, `verify_blender`, `blender`, and `recover`. `apply` accepts a typed `ReferenceCommand`; edits return changed component IDs, validation diagnostics, and a new revision. `solve` applies only explicit driving dimensions supported by the deterministic parameter solver. Other declared constraint relations are validated but are not presented as a general nonlinear CAD solver.

`render` writes the orthographic views declared by the GeometrySpec layout, the technical SVG, projection and geometry reports, direct pixel-grid material illustrations, and separate `{view}_silhouette.png` files. The current workflow uses only side, top and front; a legacy sheet may retain its historical 3/4 view. Silhouettes are a single ink color on a paper background, rasterized from the validated projection mask at 1440×480; they are built from coded profiles and never use image generation. The manifest's active `silhouette_reference` points to `side_silhouette.png`. SVG and PNG are outputs; the GeometrySpec and saved face-loop proxy remain the source. Triangles used for projection masks are derived data, not saved editable faces. Artistic relative units are not physical measurements.

`blender` creates a new staged scene, reopens it in a separate Blender process, checks evaluated component geometry and the views declared by the technical layout, and optionally publishes to `blender/models/<category>/<subcategory>/<slug>/<slug>.blend`. Publication also stores a manifest and immutable technical-reference revision under `assets/<slug>/`. An optional `design_reference_relative` records a project-local design brief with its SHA-256. Existing unpinned scenes require `expected_previous_sha256`; later publications compare the manifest-pinned scene hash. `verify_blender` inspects a saved scene without saving it. Its optional `detail_tolerance` allows bounded extra mesh detail while preserving required component bounds and exterior silhouette. It does not infer a new GeometrySpec from Blender edits.

The `blender` request can also supply `appearance_relative` and `appearance_brief`. This imports an existing project-local raster; it does not invoke a generator. The bridge bounds the image size, registers visible pixels to the side projection, clips to the authoritative geometry contour, and persists the original bytes, clipped PNG, request and report inside the immutable technical-reference revision. Input hashes are checked around the Blender process. The report is `PASS_WITH_WARNINGS` with `semantic_appearance: NOT_VERIFIABLE`: clipping proves the exterior mask only, not that internal materials and details are correct. Design, silhouette and graphic references are versioned separately in the manifest; a geometry-only publication advances the silhouette reference and leaves no active graphic reference.

Example in PowerShell:

```powershell
$env:DD_MAGIC_PROJECT_ROOT = 'D:\GitHub_rep\DD_3DMagic\demo_project'
& 'D:\GitHub_rep\DD_3DMagic\target\debug\dd-3dmagic.exe' reference requests\create_shotgun.json
```

The included request fixtures demonstrate creation, publication, and saved-scene verification. Check the JSON result for actual gate status; a draft or `NOT_VERIFIABLE` report is never a pass. Blender 4.5.0 is the locally verified version for this workspace.

## 3. Предлагаемый расширенный inventory (НЕ реализован в v0.1)

| Namespace | Operation | Role | Scope | Expected evidence |
|---|---|---|---|---|
| `asset` | `inspect` | read | asset | scene + files + locks |
| `asset` | `snapshot` | stage | asset | revision/hash |
| `asset` | `diff` | read | revisions | changed faces/layers/paths |
| `asset` | `approve` | commit | staged revision | explicit artist sign-off |
| `geometry` | `blockout` | stage | selected parts | clean mesh, orthographic views |
| `geometry` | `edit_component` | stage | allowlisted parts | topology/lock comparison |
| `uv` | `unwrap` | stage | selected mesh faces | islands, output UV hash |
| `uv` | `pack` | stage | selected islands | overlap/padding/TD report |
| `uv` | `rasterize_masks` | read/stage | atlas | island/face/material masks |
| `texture` | `import_source` | stage | art source | file hash, role, license |
| `texture` | `generate_source` | external/stage | art source | provider ID, prompt, refs, cost |
| `texture` | `project_views` | stage | UV target | camera and confidence masks |
| `texture` | `compose` | stage | atlas + masks | editable layer graph + PNG |
| `texture` | `pixel_cleanup` | stage | region | before/after cluster report |
| `texture` | `seam_pad` | stage | UV seams | seam delta & bleed tests |
| `material` | `assign` | stage | named material slots | shader diff |
| `preview` | `render` | read/stage | asset | multi-view comparison images |
| `export` | `godot_asset` | commit/export | approved asset | glb, textures, import advice |

Do not advertise a planned operation as existing. MCP `tools/list` must reflect actual implementation, not the table.

## 4. Capability, IO and error contract (future)

Every mutating operation must declare:

```json
{
  "operation_id":"unique opaque ID",
  "name":"texture.compose",
  "version":1,
  "asset_uid":"shotgun_001",
  "source_revision":"sha256:...",
  "required_capabilities":["uv-mask-raster", "image-compose"],
  "input_artifacts":[{"role":"approved_motif","uri":"asset://...","sha256":"..."}],
  "target_scope":{"component_uids":["stock"]},
  "expected_locks":{"untouched_uids":["barrels"]},
  "profile_snapshot":"sha256:...",
  "write_mode":"stage_new",
  "outputs":{"artifact_dir":".assetlab/staging/job-id"}
}
```

Success includes: generated artifact paths, software/provider versions, input hashes, param snapshot, validation reports and preview links. Failure includes stable code (`MISSING_UV`, `CONTEXT_CONFLICT`, `LOCK_VIOLATION`, `TD_CAPACITY`, `UNSUPPORTED_PROVIDER`, `OUTPUT_EXISTS`, `INVALID_PATH`), evidence, no silent partial writes. NEVER send `stdout` logs from MCP as this corrupts JSON-RPC.

## 5. Operation transaction state

- `plan`: inspect source revision, locks and profiles, calculate scope, return expected changes before execute.
- `stage`: create new folder/job id and generated outputs; no approved originals touched.
- `validate`: mechanical checks + visually inspectable previews. A valid texture is not automatically artist-approved.
- `review`: compare and accept/reject; safe by default.
- `commit`: replace logical asset **reference** with approved new revision, never mutate historical source.
- `rollback`: point to last approved revision; report any externally modified Blender file separately.

An operation must be idempotent by `(inputs, params, tool version)` once its image sources are frozen; AI creative sampling cannot be assumed reproducible and must be materialized before deterministic processing.

## 6. Locks and dependency invalidation

Geometry lock, UV lock, layer/region lock, and reference version lock are distinct. Phase 1 records declared source/reference hashes and reports drift, but it does not compute `mesh_signature`, `uv_signature`, or `layer_stack_revision`, map locks to Blender faces, or invalidate dependent outputs.

## 7. Blender bridge

Current: standalone `scripts/blender/export_uv.py` invoked manually in background Blender, read-only `.blend`, writes JSON. Next stage: typed adapter with dispatch `scene.inspect`, `uv.extract`, `material.inspect` and explicit review-gated modify methods. No generic `execute_arbitrary_code` MCP tool in release mode. Blender Python runs in Blender process; safety policy must account for user-authored .blend scripts and network environment.

## 8. Integration protocols

Future `ImageProvider` interface: `capabilities()`, `generate(request, reference_assets)`, `edit(request, input_image, mask)`, `estimate_cost()`, `cancel()`. Must not treat prompt mask as precise UV region. Future `Exporter` interface: `export(asset_revision, destination_profile)` and engine-specific material/import verification. UI, MCP and CLI call domain commands, not the other way around.
