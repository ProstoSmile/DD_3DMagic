# Длинное ружьё · рабочая база v1

| Этап | Артефакт | Статус |
| --- | --- | --- |
| Силуэт | [Лист привязки](../../references/design/orthographic_alignment_v1/alignment_sheet.png), [штрихи и контрольные точки](../../references/design/orthographic_alignment_v1/baseline.json) | Принятая художественная основа. Верх и бок зарегистрированы по одной продольной оси. |
| Graphic Reference | [Бок](graphic/side.png), [верх](graphic/top.png), [фронт](graphic/front.png), [промпты](graphic/graphic_reference.json), [входные изображения](graphic/inputs), [пример крупности текселей](graphic/texel_scale_sample.png) | Ориентир по внешнему виду объёмной модели. 30 px/m — целевое значение, визуально приближённое генерацией. |
| Модель | [Проверенная сцена](model/scene.blend), [GeometrySpec](model/geometry.json), [кадры этапов](model/model_checkpoints.png) | Ревизия 6, 9 отдельных мешей, 308 вершин, 584 треугольника. |
| Сверка | [Наложение](model/projection_centerline_review.png), [отчёт Blender](model/comparison.json), [пробный фронт](model/front_provisional_overlay.png) | Blender повторно открыл сцену и подтвердил три проекции относительно GeometrySpec. С рисунком совпало 94,4% сбоку и 94,0% сверху. Увеличенный фронт остаётся визуальным ориентиром. |
| Более позднее сохранение | [Рабочая сцена](working_edit/scene.blend), [фактические объекты](working_edit/inspect_report.json), [боковой кадр](working_edit/blender_side.png) | 8 мешей. Отдельного объекта `receiver` нет, геометрия `stock` отличается. MCP отклонил совпадение с GeometrySpec ревизии 6. |

Проверенная сцена восстановлена в пакет копированием резервной версии Blender с совпадающим SHA-256. Более позднее сохранение скопировано отдельно. Активная сцена демопроекта осталась прежней.

В [handoff.json](handoff.json) записаны хэши материалов пакета и ссылка на силуэтную базу. Снимки скриптов сохранены в `recipe/`; рабочие команды запускаются из `demo_project/requests/`. Этапы моделирования: `01_masses`, `02_assembly`, `03_refined`. Процесс описан в [руководстве](../../../../../docs/08_ASSET_WORKFLOW.md).

Для UV и измерения TD сначала зафиксируй масштаб в метрах и выбери конкретную сцену, которую предстоит разворачивать.
