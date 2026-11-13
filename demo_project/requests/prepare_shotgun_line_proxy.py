"""Prepare the explicit GeometrySpec for the black ink shotgun study."""

import hashlib
import json
from pathlib import Path

project = Path(__file__).resolve().parents[1]
source = ".assetlab/staging/shotgun_line_study/07_level_detail.json"
drawing_bytes = (project / source).read_bytes()
drawing = json.loads(drawing_bytes)
outline, forearm, guard_outer, guard_inner, trigger, sight, butt_seam = [
    stroke["points"] for stroke in drawing["strokes"][:7]
]
scale = 0.022
origin_x, origin_y = 185, 58


def node(id, label, material, primitive, *, y=0, tx=0, z=0):
    return {
        "id": id, "label": label, "material": material, "children": [],
        "primitive": primitive,
        "transform": {
            "translation": {"x": tx, "y": y, "z": z},
            "rotation": {"x": 0, "y": 0, "z": 0},
            "scale": {"x": 1, "y": 1, "z": 1},
        },
    }


def profile(id, label, material, points, depth, *, y=0):
    # Drawing Y increases down. Geometry Z increases up.
    coordinates = [[(x - origin_x) * scale, (origin_y - z) * scale] for x, z in points]
    return node(id, label, material, {
        "kind": "extruded_profile",
        "parameters": {"profile": coordinates, "depth": depth, "axis": "y"},
    }, y=y)


def barrel(id, label, material, stations, *, bore):
    start_x = stations[0][0]
    contour = [[(x - start_x) * scale, radius * scale] for x, radius in stations]
    # The inner return closes the lathe profile without zero-area pole faces.
    inner_radius = bore * scale
    contour.extend([[contour[-1][0], inner_radius], [0, inner_radius]])
    return node(id, label, material, {
        "kind": "revolve",
        "parameters": {"profile": contour, "axis": "x", "segments": 48},
    }, tx=(start_x - origin_x) * scale, z=(origin_y - 51) * scale)


def main():
    candidate = "v3"
    seam = [[23, 46.33], *butt_seam[1:]]
    forearm_top = [[276, 55], [266, 55], [230, 56], [192, 56]]
    guard_outer_join = [[133, 74], *guard_outer[1:-1], [182, 66]]
    guard_inner_join = [[145, 70], *guard_inner[1:-1], [176, 67]]
    trigger_join = [[156, 69], *trigger[1:-1], [162, 69]]
    root = node("shotgun", "Long shotgun from ink study", "steel", None)
    root["children"] = [
        profile("stock", "Carved stock", "wood",
                seam[:1] + outline[2:10] + [[121, 48], [130, 78]]
                + outline[40:50] + seam[:0:-1], 0.32),
        profile("buttplate", "Buttplate", "dark_steel",
                seam + outline[50:-1] + outline[:2], 0.33),
        profile("receiver", "Steel receiver", "steel",
                outline[9:16] + outline[33:41] + [[119, 72], [116, 63], [116, 55]], 0.41),
        profile("receiver-side-plate", "Side plate", "dark_steel",
                drawing["strokes"][11]["points"], 0.035, y=-0.209),
        barrel("barrel", "Long tapered barrel", "steel",
               [(187, 6), (221, 6), (260, 5), (311, 5), (359, 4), (382, 4)], bore=2.7),
        barrel("muzzle", "Muzzle edge", "dark_steel",
               [(378, 4), (380, 5), (385, 5), (387, 3)], bore=2.4),
        profile("forearm", "Carved forearm", "wood",
                forearm[:-1] + forearm_top, 0.35),
        profile("trigger-guard", "Trigger guard", "dark_steel",
                guard_outer_join + guard_inner_join[::-1], 0.16),
        profile("trigger", "Curved trigger", "dark_steel", trigger_join, 0.08),
        profile("front-sight", "Front sight", "dark_steel", sight + [[373, 48], [366, 48]], 0.08),
    ]
    previous = "assets/long_shotgun/references/technical/r2_5f7ac220e7be/geometry.json"
    spec = json.loads((project / previous).read_text(encoding="utf-8"))
    spec.update(name="Long Shotgun from Ink Study", revision=spec["revision"] + 1, root=root)
    spec["parameter_provenance"] = [
        {"path": "*", "basis": "artistic",
         "explanation": "Widths and cross sections are artistic estimates in relative units."},
        {"path": "root.children", "basis": "derived",
         "explanation": f"Side contours trace {source} (SHA-256 {hashlib.sha256(drawing_bytes).hexdigest()})."},
    ]
    spec["driving_dimensions"] = [
        {"id": "receiver-width", "label": "Receiver width", "unit_label": "au",
         "assignment": {"node_id": "receiver", "parameter": "depth", "value": 0.41}},
    ]
    request = {"operation": "create", "spec": spec,
               "output_relative": f".assetlab/staging/shotgun_line_proxy_{candidate}"}
    output = project / f".assetlab/staging/shotgun_line_proxy_request_{candidate}.json"
    with output.open("x", encoding="utf-8") as file:
        json.dump(request, file, indent=2)


if __name__ == "__main__":
    main()
