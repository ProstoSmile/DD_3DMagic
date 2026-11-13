# DD_3DMagic — CRT Workstation UI System

## 1. Образ и цель

«1990s workstation / photo retouching laboratory»: Photoshop-подобное расположение инструментов и непрерывный editable document, но внешний вид старого качественного CRT дисплея, а не дешёвого фильтра хоррор-игры. Это инструмент для долгой рабочей сессии, поэтому ошибки читаемости недопустимы.

## 2. Токены

| Token | Hex | Use |
|---|---|---|
| `screen.bg` | `#0A1212` | outside canvas |
| `panel.base` | `#101D1C` | dock/toolbar container |
| `surface.raised` | `#203931` | hover/selected control |
| `text.primary` | `#C6DEB6` | label, tool output |
| `text.muted` | `#739984` | annotations, paths |
| `accent.phosphor` | `#7ED093` | focus, selected, OK |
| `warning.amber` | `#E3B865` | destructive/unverified/warning |
| `danger.coral` | `#CE7267` | actual failure |

Borders: default 1px low-contrast; active control may use 2px accent but never rely on color alone. Font: OS-provided clean monospace for metadata/metrics, legible UI sans for large editor text if needed. No redistributed font files. Spacing scale 4/8/12/16/24px. Max 2 hierarchy levels per panel plus collapsible groups. Icons and text must be distinguishable in grayscale. **No unnecessary futuristic chrome.**

## 3. Photoshop logic

Top: project/workspace / document tabs / file operations. Left: tool categories (select, pan, mask, sample, pixel operations). Center: content; multi-document tab in later phase. Right: layers, channel/material/UV assignment, Inspector for selected object/face/operation. Bottom: status, errors, operation history. This mirrors professional editing priority: canvas first, controls nearby, details only when asked.

For v0.1 current implementation uses static (resizable) `egui` side panels, one canvas, bottom log and real operation stack. No fake brush, no fake paint layers. The location/size of current UI panel does not imply dynamic docking is complete.

## 4. Future document workspaces

- `TEXTURE`: raster canvas, layers, masks, crop/palette tools, zoom, per-pixel inspection.
- `UV`: UV island view atop active atlas, island IDs, seam highlighting, actual TD heatmap, protect selection.
- `MESH PREVIEW`: Blender-backed render, not proprietary full-featured modeling viewport. View presets: front/side/top/3/4.
- `REFERENCE`: master reference comparison and verified dimensions.

Each uses the same selection model and document identity. An asset may have separate texture/material/mesh docs simultaneously.

## 5. CRT effects, separated from images

Optional later: 1px faint bezel/reflection at application edge, very subtle panel scanlines at low opacity, soft glow only on focused interface labels. **All** disabled automatically in pixel-accurate canvas and screenshot/export. No distortion/scanlines/bloom applied to art source, UV maps, output PNG or Blender screenshots used for texture validation. Default current app renders no scanline noise.

## 6. Workflow microinteractions

- Drag/drop PNG to edit. Hover tool shows exact scope/side effects.
- Applying operation immediately changes view, not original source.
- UI always labels imported raster, active operation stack and profile provenance.
- Mutating UV would show a side-by-side diff + protected-region badges before accept (future).
- Contradictions produce explicit choice; never automatically choose lower TD/overwrite/pack.
- Keyboard: Ctrl+Z step undo, Ctrl+S new preview export (current app). Later Ctrl+Tab documents, B brush, G gradient, M mask, V move, 1–4 previews, adjustable binding profiles.

## 7. Anti-patterns

Don't split every pixel of viewport into CRT boxes; don't make fluorescent green 100% of text; don't put a command console where normal interactive controls belong; don't bury locks or TD warnings; don't invent an empty Photoshop clone with 70 disabled buttons; don't duplicate core logic in UI. Artist should operate without knowing Codex internals.
