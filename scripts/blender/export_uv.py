"""Read-only UV inspection bridge for Blender 5.x.

Usage (from repository root):
  blender -b demo_project/assets/demo_prop/sources/model.blend \
    --python scripts/blender/export_uv.py -- \
    --out demo_project/.assetlab/staging/uv_dump.json

Script never changes .blend. It writes *one* requested JSON file (fails if it exists).
DD_3DMagic MCP does not run this script automatically in v0.1.
"""

import argparse
import json
import os
import sys
from pathlib import Path

import bpy


def parse_args():
    raw = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    parser = argparse.ArgumentParser()
    parser.add_argument("--out", required=True)
    parser.add_argument("--uv", default=None, help="UV map name, default active")
    parser.add_argument("--selected", action="store_true", help="Only selected mesh objects")
    return parser.parse_args(raw)


def collect(uv_name=None, selected=False):
    triangles = []
    graph = bpy.context.evaluated_depsgraph_get()
    scale = bpy.context.scene.unit_settings.scale_length
    # Reject non-metric scene unit configuration instead of guessing units.
    if bpy.context.scene.unit_settings.system != "METRIC":
        raise ValueError("Scene unit system must be METRIC for meters-based TD analysis")
    if scale <= 0:
        raise ValueError("Scene unit scale must be positive")
    for obj in bpy.context.scene.objects:
        if obj.type != "MESH" or (selected and not obj.select_get()):
            continue
        eval_obj = obj.evaluated_get(graph)
        mesh = eval_obj.to_mesh(preserve_all_data_layers=True, depsgraph=graph)
        if mesh is None:
            continue
        try:
            uv_layer = mesh.uv_layers.get(uv_name) if uv_name else mesh.uv_layers.active
            if uv_layer is None:
                raise ValueError(f"Object {obj.name} has no required UV map")
            mesh.calc_loop_triangles()
            world_matrix = eval_obj.matrix_world
            for tri in mesh.loop_triangles:
                pos = []
                uv = []
                for loop_idx in tri.loops:
                    idx = mesh.loops[loop_idx].vertex_index
                    pt = world_matrix @ mesh.vertices[idx].co
                    pos.append([float(v) * scale for v in pt])
                    uv.append([float(v) for v in uv_layer.data[loop_idx].uv])
                triangles.append({"object": obj.name, "positions_m": pos, "uv": uv})
        finally:
            eval_obj.to_mesh_clear()
    return {"schema_version": 1, "triangles": triangles}


def main():
    args = parse_args()
    output = Path(args.out).expanduser().absolute()
    if not output.parent.is_dir():
        raise FileNotFoundError("Output parent directory does not exist")
    data = collect(args.uv, args.selected)
    if not data["triangles"]:
        raise ValueError("No mesh triangles exported")
    # Exclusive creation, no replacement of previous inspections.
    with output.open("x", encoding="utf-8") as handle:
        json.dump(data, handle, separators=(",", ":"), ensure_ascii=False)
    print(f"Exported {len(data['triangles'])} UV triangles to {output}")


if __name__ == "__main__":
    main()
