# Исследование открытых источников и обоснование технологий

**Проверено:** 2026-11-10. Источники внизу — публичная документация или открытые репозитории. Анализ затрагивает структуру проектов, возможности и ограничения, не только отдельные кодовые примеры. Ссылки нужны агенту для повторной верификации при обновлении зависимостей. Код сторонних лицензированных аддонов сюда не копировался.

## 1. Desktop editor: egui/eframe против Tauri/Electron и тяжёлых UI-stack

- `eframe` + `egui`: нативная immediate-mode библиотека, быстрые частые UI-инспекции и canvas, меньше инфраструктуры чем web-front/backend. **Выбран** `eframe 0.36.2`, `glow` для prototyping. К ноябрю 2026 существует `0.37`, но latest docking-библиотека `egui_dock 0.21.1` на `egui 0.36`, поэтому версии надо контролировать, а docking не включать до интеграционного теста. Позже вероятно `egui_dock` или `egui_tiles`. [eframe releases](https://docs.rs/crate/eframe/latest), [egui_dock](https://docs.rs/crate/egui_dock/latest), [egui](https://github.com/emilk/egui).
- `Tauri`: полезен для зрелого HTML/CSS интерфейса, но добавляет web renderer и границу JS/Rust в core drawing workflow. Не выбираем для минималистичного первого прототипа, особенно пока нет фактической потребности в сложном DOM.
- `Electron`: ещё большее runtime основание ради текстурного canvas, не соответствует lightweight-цели.
- `iced`: допустимая альтернатива для general app; здесь удобнее `egui`, где canvas/inspector и update-driven tooling естественнее. Выбор не считается безусловно лучшим для Photoshop-аналога: если позже тяжёлый layer compositor начнёт упираться в GUI, потребуется объективный performance audit.
- Для pixel preview используется `egui::TextureOptions::NEAREST`. Он управляет **только отображением**, не является заменой пиксельной художественной обработке. [egui TextureOptions](https://docs.rs/egui/0.36.2/egui/struct.TextureOptions.html).

**Решение:** UI is shell, state and operations in Rust core, no dependence on active Codex/MCP/Blender to display a saved PNG. CRT effect is isolated UI theme.

## 2. MCP и Codex: протокол отдельно от процедур

- [Официальный Rust SDK RMCP](https://github.com/modelcontextprotocol/rust-sdk): в актуальной ветке есть 3.x, stdio transport, tool-router, JSON schemas и совместимость с современной MCP-спецификацией. [Examples](https://github.com/modelcontextprotocol/rust-sdk/tree/main/examples/servers) содержат counter stdio и memory stdio, отдельно организованы common services и protocols. Для DD_3DMagic выбран RMCP, локальный `stdio`, типизированные инструменты. На каждый tool должен существовать публичный контракт; чтение и запись следует разграничивать.
- [MCP 2026-07-28 release](https://blog.modelcontextprotocol.io/posts/2026-07-28/): современный transport/protocol стал сложнее, поэтому нет смысла писать собственный «JSON-RPC-почти-MCP» протокол.
- [Codex customization](https://learn.chatgpt.com/docs/customization/overview) и [Build skills](https://learn.chatgpt.com/docs/build-skills): `AGENTS.md` — долговечные правила репозитория; `.agents/skills/*/SKILL.md` — отдельные процедуры с progressive disclosure; MCP — инструменты. Skills не должны дублировать JSON schemas/данные и не должны быть однообразными копиями гигантского промпта.

**Решение:** Phase 1 предоставляет 6 реализованных MCP tools: discovery проектов и assets, resolve context, форматирование имён, UV-анализ и staging растра. Остальной каталог в `03_API_OPERATIONS.md` остаётся **планируемым**. Skills оркестрируют доступные capabilities и не выдают задуманные операции за реализованные.

## 3. Blender Python API и стабильность UV-данных

- [Blender 5.0 Python API](https://docs.blender.org/api/5.0/) и [Mesh](https://docs.blender.org/api/5.0/bpy.types.Mesh.html): `mesh.calc_loop_triangles()` / `loop_triangles`, `uv_layers.active`, `tri.loops` и `uv_layer.data[loop_index].uv` дают корректную связь corners и UV. Растровый атлас нельзя строить на голых vertex UV: одна вершина может иметь разные UV на смежных гранях.
- [BMesh UV layer API](https://docs.blender.org/api/5.0/bmesh.html): доступ к UV через loops, а не как одно значение вершины; важен для seam topology.
- [Blender 5.0 API changes](https://developer.blender.org/docs/release_notes/5.0/python_api/): менялись свойства UV pin/selection; нельзя без проверки переписывать скрипты под старый API.
- [Cycles Baking](https://docs.blender.org/manual/en/5.0/render/cycles/baking.html) (альтернативный перевод: [ru](https://docs.blender.org/manual/ru/5.0/render/cycles/baking.html)): есть margin типа Extend и Adjacent Faces; bake-target активная Image Texture нода; margin нужен также при mipmaps/filtering. Значит, «увеличить острова» и «запечь с padding» — не одна операция.

**Решение:** сначала read-only экспорт world-space UV triangles с учётом evaluated modifiers и scene units; затем Rust analyzer. Не терять семантику в глобальном UV dump.

## 4. Существующие алгоритмы UV

- [xatlas](https://github.com/jpcy/xatlas): зрелая библиотека генерирует UV charts и packing; есть раздельные ComputeCharts и PackCharts, управление texels per unit / resolution. Это важное подтверждение, что unwrap/packing стоит держать заменяемым модулем. Не внедрять C++ FFI в первый день ради алгоритма, который пока можно исполнить через Blender. Учитывать лицензии и отдельное тестирование FFI при выборе.
- [Pixel UV Tools](https://github.com/Capacap/pixel-uv-tools): проект целиком посвящён pixel-perfect UV; snap-to-grid, центрирование и packing с учётом реального разрешения, частично subpixel islands. Существование таких инструментов подтверждает, что идеальный average TD недостаточен для низкого разрешения. У аддона GPL-3.0: анализировать идеи можно, копирование/линковка требуют отдельной юридической оценки.
- [Pixel Art Studio](https://pardalltools.github.io/pixel-art-studio-docs/): опубликованный workflow density zones, stackable non-destructive adjustments, masks и nearest filtering, seam padding. Документация честно указывает, что auto-UV подходит не для всех моделей. Значит, требовать «один алгоритм автоматически правильно разворачивает любой mesh» бессмысленно.
- [Texture filtering & seam padding](https://pardalltools.github.io/pixel-art-studio-docs/texture-filtering.html): nearest для hard pixel appearance; padding / linear требуют иного запаса вокруг островов, можно отследить документированно.

**Решение:** adapter interface для Blender native unwrap / pixel-tool addons / later xatlas; базовая технология не привязана к конкретному платному или GPL аддону.

## 5. Photoshop/Krita и важность non-destructive operations

- [Krita Filter Layers](https://docs.krita.org/en/reference_manual/layers_and_masks/filter_layers.html): фильтр-слой меняет представление слоя под ним без прямого перезаписывания исходного растра; маски и группировка задают область применения. Это практический образец для нашего `RasterDocument{original,operations}` и будущего compositing graph.
- Следующее архитектурное следствие: Photoshop-like не означает все инструменты сразу. Сначала `Document`, `Layer`, `Operation`, `Mask`, `Selection`, `Viewport`, `History`. На этапе v0.1 реализован лишь исходный растр + **стек операций**, не настоящий multi-layer compositor.

## 6. Генеративное изображение — художественный источник, не UV-координатор

- [OpenAI Image Generation Guide](https://developers.openai.com/api/docs/guides/image-generation): редактор может выдавать художественные источники/варианты и работать по нескольким референсам, но маска трактуется через prompt. Точность границ не гарантируется. [Image Edit API](https://developers.openai.com/api/reference/resources/images/methods/edit) поддерживает явные параметры, меняющиеся модели, прозрачные outputs, и зависит от доступа/тарификации API.
- **Не закреплять в коде конкретную модель/API-провайдера**. В проекте должны быть `ImageProvider` + `LocalImageSource` + асинхронная задача + provenance: model id, prompt, reference IDs, file hash, user approval. Отправляемые в сервис изображения должны согласовываться по приватности и затратам.

**Решение:** генератор только формирует source art; точные UV-маски/выпечка/пиксельная сетка — код. v0.1 не делает никаких сетевых вызовов и не требует API key.

## 7. Godot как export consumer

- [Godot image import](https://docs.godotengine.org/en/latest/tutorials/assets_pipeline/importing_images.html): Detect 3D может включать mipmaps и VRAM compression. Документация отмечает, что даже в 3D pixel-art текстуры могут страдать от VRAM compression, и для них часто нужен Lossless; filtering в 3D определяется материалом, не PNG.

**Решение:** будущий экспортёр должен проверять финальное качество **в Godot**, хранить import advice и shader/sample settings. Не копировать шаблон импорта от обычного PBR-ассета. Настройки компрессии и mipmaps — profile-dependent, не глобальные непреложные истины.

## 8. Риск Blender MCP

- [ahujasid/mcp-for-blender](https://github.com/ahujasid/mcp-for-blender) — рабочий open-source ориентир для связи LLM/Blender; проект имеет отдельные addon/socket/Python server, scene inspection, material changes и raw Python execution. В открытом репозитории есть описание safe mode; это **не настоящий sandbox**: сам сокет всё равно может принимать произвольный код. См. [safe_mode.py](https://github.com/ahujasid/mcp-for-blender/blob/main/src/blender_mcp/safe_mode.py) и [README security](https://github.com/ahujasid/mcp-for-blender/blob/main/README.md).

**Решение:** для быстрого прототипа допускается внешний Blender MCP, но собственный доверенный сервер должен давать только scope-bounded tools. Нельзя выставлять Blender raw-exec в сеть. Учитывать смену названия/версий пакета и проверять установленную версию при подключении.

## 9. Вывод по исследованию

Архитектура должна признавать три разных класса ошибок: **художественные** (генерация неверного дизайна), **геометрические** (слишком простой/неправильный mesh или плохой unwrap), **инженерные** (неверные координаты проекции, padding, цветовое пространство, повреждённые файлы). Если все три смешаны в одном промпте, диагностика невозможна.

Порядок внедрения: project/asset context → UV inspection → deterministic compositor → non-destructive preview/commit → image generation adapters → view projection → broader geometry generation.

**Неисследованное / требующее реальных тестов:** поведение Blender 5.x на конкретных пользовательских мешах, совместимость `egui 0.36.2` с Windows-драйверами, фактический GPU/CPU бюджет тяжёлой UV-composition, точность любых image models для многоракурсной проекции, корректность предварительной оценки packing-efficiency. Эти неопределённости идут в acceptance tests, а не маскируются архитектурным текстом.
