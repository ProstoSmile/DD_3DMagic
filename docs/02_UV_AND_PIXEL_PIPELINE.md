# UV / pixel-art texture pipeline — инженерная спецификация

## 1. Входной контракт

Для любой UV-aware операции требуются: topology snapshot, world-space triangle vertices (m), UV per *face corner*, atlas resolution, UV map name, asset/component/material IDs, transform units. Опционально: seam edges, locked UV sets, approved layer sources, normals, cameras, high-res projections. Missing UV map is **error**, unless user explicitly requests `uv.unwrap`.

Учитывать object/world transforms, nonuniform scale, evaluated mesh modifiers и scene scale. Для bake нужен идентификатор именно UV version, с которой связан texture atlas. Изменение UV topology invalidate `texture.compose` output, пока не прошёл reproject/review.

## 2. Как измеряется TD

Для треугольника мировая площадь `A3D` [m²], площадь в UV `Auv`, `W,H` — разрешение изображения. Для квадратного atlas: приблизительная плотность `D=R sqrt(Auv/A3D)` [px/m]. Для прямоугольного atlas: `sqrt(W*H * Auv/A3D)` [px/m]. Это геометрическое среднее двух направлений, а не полный критерий растяжения.

Для физически корректного анализа строим локальный ортонормированный базис треугольника: вектор вдоль p1−p0 и перпендикуляр в касательной плоскости. Затем берём матрицу Якоби `J` преобразования локальных мировых координат в pixels-space (`u*W`,`v*H`). SVD даёт `sigma_min`, `sigma_max` [px/m]. `sqrt(abs(det J))` — geometric mean TD, `sigma_max/sigma_min` — UV anisotropy. В core v0.1 реализованы данные показатели на каждом треугольнике.

Не смешивать показатели: планируемый `target_td_px_per_m` может быть выполнен по площади при недопустимом `anisotropy_ratio`. Нужно отдельно помечать degenerate faces, huge anisotropy, flipped UV (не всегда ошибка), UV outside 0..1 (UDIM possible), overlap policy. `uv.analyze` текущей версии не проверяет overlaps/UDIM/packing.

### Capacity gate

Теоретическая нижняя оценка количества уникальных текселей: `Σ(A3D_i × Dtarget_i²)`. Реальная потребность выше из-за inefficient packing, padding, empty space, duplicate faces и дискретизации островов. При intentional stacked/mirrored UV нельзя просто суммировать всё без учёта shared coverage. Capacity estimator должен явно знать `overlap_policy` и выбранный packer; никакой выдуманной универсальной константы «эффективность 80%».

Пример: 0.8 m² уникальной поверхности при 128 px/m потребует >= 13,107 texels, тогда как atlas 64×64 содержит всего 4096. Это несоответствие надо обнаруживать до GPU bake. `TD=128` не значит, что любая геометрия автоматически помещается в `128×128`.

## 3. Семантика ассета

Material regions и selected faces — разные вещи. В будущей схеме: `component_uid`, `face_uid`, `material_slot_uid`, `uv_island_uid` хранятся отдельно; UI имена декоративны и не служат первичными ID. Если topology меняется, `face_uid` может потребовать remapping; нельзя считать имена или индексы полигонов бессмертными.

`TargetScope` должен указывать какие exact face regions/UV masks затрагиваются; `preserve_scope` — кто lock. Векторные boundary masks генерируются из UV, а не из ImageGen. Supersampling допустим как промежуточный anti-aliasing art source, но финальный pixel atlas создаётся на target pixel grid.

## 4. Три взаимозаменяемых художественных метода

### A / Material-first

Input: semantic region+mask, approved wood/metal/leather source, orientation vector in surface space, density, reference palette. Pipeline: tile/sample motif → domain warp (controlled) → UV triangle rasterization → region mask → layer stack → seams. Древесные волокна не должны идти в произвольном UV направлении: orientation задаётся художником или inferred от геометрической продольной оси части с confidence score.

### B / View-to-UV projection

Render exact ortho/front/side/3-4 of actual mesh with stable camera matrices; external art generator may edit *rendered appearance*, not UV chart. В Blender/Rust для texel `(u,v)` определяется world surface point через barycentric inversion на соответствующем UV triangle. Затем camera projection → source image sample, глубина/normal/visibility test → confidence weight. Для occluded texels нужны альтернативные камеры, procedural fill или manual selection. Multiple views не смешивать простым средним: appearance/art model может изменить расположение детали. Следует хранить source camera, segmentation mask, estimated confidence, UV target hash. Для повторной projection инструмент должен использовать одобренные изображения, не регенерировать их.

### C / UV-space compositing

Растеризатор заполняет texel masks: `island_id`, `face_id`, `material_region_id`, `coverage`, optional `world_pos`, `normal`, `camera_weight`. При overlaps распределение texel нескольких faces вызывает конфликт, если отсутствует declared mirroring/stacking policy. Composition выполняется как последовательность слоёв с blend modes, opacity и selected mask. Все швы и padding проверяются после композиции на mesh preview.

## 5. Real pixel-art

Низкое разрешение ≠ грамотный pixel-art. Нужны: дизайн формы до дискретизации, ограничение значимых tonal ramps, pixel cluster cleanup, запрещение единичного шума, hand-guided edge highlights, shadow grouping и единая художественная шкала между материалами.

Предлагаемая внутренняя последовательность:

1. Художественный source в высоком или промежуточном разрешении → artist approval.
2. Семантические region masks в UV coordinates на target resolution.
3. Преобразование материала/проекции в target raster при фиксированном pixel grid.
4. Палитра: запрет случайных цветов; perceptual clustering Lab/OKLab или approved palettes на будущем этапе. Текущее `palette_nearest` использует RGB distance **как технический примитив**, не финальный художественный алгоритм.
5. Cluster cleanup с запретом исчезновения тонких смысловых элементов. Запрет слепого «удалить всё одиночное».
6. Shadow/highlight/lut accents не в обход locked regions.
7. Color-space-aware conversion (sRGB vs linear для освещения, base color output обычно sRGB). Не смешивать обычный pixel RGBA и physically-correct baked lighting.
8. Seam-aware padding с защитой соседних UV masks.
9. Nearest render preview + actual engine import preview.

Мипмапы: в 3D нужны часто для расстояний; но mip sampling/atlas bleeding проверяются вместе с padding и целевым движком. «Nearest» не магический способ избавиться от bleed. Для Godot pixel textures полезна Lossless compression, а не автоматическая VRAM compression, см. research docs.

## 6. Порядок UV операций

Если existing UV approved: inspect, calculate TD; if fail — report, no auto unwrap. Если UV absent and user explicitly requested preparation: unwrap adapter selected by mesh category; preserve seam hints; normalize scale; check TD capacity; pack with margin measured in real pixels; snap where pixel-art profile permits; calculate metrics again; preview checkerboard on model.

`uv.pack` обязан поддерживать `preserve_island_orientation`, `allow_mirror`, `allow_stack`, `udim`, `locked_islands`, `padding_px`, `target_resolution`. Алгоритм упаковки (Blender native / addon / xatlas) выбирается capability policy, not hardcode.

## 7. Seam handling

Extending each island's outermost texel полезно, но иногда создаёт ступеньки на границе непрерывной поверхности. `Adjacent Faces` в Blender bake использует соседние faces across seams; для pixel art важно ещё matching cluster directions, а не только edge color. При material boundary color continuity не всегда нужна. Сравнивать seam pairs по world adjacency и T-образным разрезам. Preview на mesh и автоматический seam delta report со снимком до/после.

## 8. Необходимые проверки

- Mesh: empty/degenerate geometry, correct units, scale, UV existence, locked component status.
- TD: per-face geometric mean; local min/max; distribution percentiles, outliers, anisotropy.
- Atlas: resolution, UV outside bounds, overlaps (separate intentional/mistake), min island width in texels, padding, effective coverage.
- Image: dimensions, alpha/ICC, palette limit, unexpected isolated pixels, source hash, no unapproved changes to masks.
- View projection: known calibrated camera, depth consistency, backface rejection, confidence, coverage holes, discontinuity warnings.
- Blender material: Image Texture interpolation `Closest`, output channel, color space, roughness/metallic profile, no extra nodes accidentally overwriting manual settings.
- Export: naming, Godot material filtering/compression policy, path relative to game project, viewport compare, manifest.

**Release v0.1 includes ONLY:** world-space TD+anisotropy analyzer from manually exported triangles, and basic independent raster transforms. The rest is technical design for implementation.

## 9. Test data / edge cases

Plan at least: cube standard UV; cube mirrored UV; bevelled prop with nonuniform object scale; thin sliver islands; intentional UV stacking; 64×64 atlas with impossible target TD; color-correct aged wood; overlapping material masks; change geometry after bake; long rectangular 512×128 atlas; UDIM mesh; high-poly model limited by tool memory; protected human paint region; normals discontinuity at seams; pixel art nearest/mips from oblique camera.

Use Blender-generated ground truth for UV values and Rust unit tests for exact TD cases. For any actual edit, compare hashes of protected geometry, UV, source layers before/after. Fail if tests do not verify claimed preservation.
