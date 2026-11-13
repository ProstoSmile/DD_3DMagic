---
name: dd-artist-ui-review
description: Review DD_3DMagic native editor UI for Photoshop-like task flow, strict hierarchy and functional CRT theme. Use when coding or auditing UI changes.
---

# UI implementation discipline
1. Read `docs/04_UI_DESIGN_CRT.md`. Keep central artwork dominant; hierarchy from panel→section→control→status. Use restrained 1px borders, spacing scale and low-contrast grouping.
2. CRT aesthetic applies to the chrome only. Never apply CRT scanlines or glow to the raster, UV preview, generated output or color judgement area.
3. Functional controls only: if brush, layer compositor, UV viewport or 3D mode is not implemented, don't draw a working-looking button that does nothing.
4. Check all changed workflows with mouse, keyboard, resizing, long file names, missing assets and dark/bright source textures.
5. Compare semantics of operations to actual core output; don't build a second processing implementation in UI.
6. Test reading at 100% zoom, selectable controls, dark-green text contrast, warnings distinguished from normal labels and status log from content.
7. Report usability issues concretely and prioritize issues blocking artist workflow over ornamental UI improvements.
