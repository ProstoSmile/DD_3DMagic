"""Smoke-test the Blender UV exporter without opening or saving a .blend file.

Run from the repository root:
    blender --background --factory-startup --python-exit-code 1 --python tests/blender_uv_smoke.py
"""

import importlib.util
import json
import math
import sys
import tempfile
from pathlib import Path

import bpy


ROOT = Path(__file__).resolve().parents[1]
EXPORT_SCRIPT = ROOT / "scripts" / "blender" / "export_uv.py"
UV_SQUARE = ((0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0))


def load_exporter():
    spec = importlib.util.spec_from_file_location("dd3d_export_uv", EXPORT_SCRIPT)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Could not load UV exporter at {EXPORT_SCRIPT}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def assign_square_uv(mesh):
    for layer in list(mesh.uv_layers):
        mesh.uv_layers.remove(layer)
    layer = mesh.uv_layers.new(name="UV_Main")
    mesh.uv_layers.active_index = 0
    for polygon in mesh.polygons:
        for corner, loop_index in enumerate(polygon.loop_indices):
            layer.data[loop_index].uv = UV_SQUARE[corner]


def build_test_scene():
    if bpy.data.filepath:
        raise RuntimeError("Run with --factory-startup; the smoke test refuses an open .blend")

    scene = bpy.context.scene
    scene.unit_settings.system = "METRIC"
    scene.unit_settings.scale_length = 1.0

    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)

    bpy.ops.mesh.primitive_cube_add(size=1.0)
    cube = bpy.context.object
    assign_square_uv(cube.data)

    plane_mesh = bpy.data.meshes.new("SmokePlaneMesh")
    plane_mesh.from_pydata(
        ((-0.5, -0.5, 0.0), (0.5, -0.5, 0.0), (0.5, 0.5, 0.0), (-0.5, 0.5, 0.0)),
        (),
        ((0, 1, 2, 3),),
    )
    plane_mesh.update()
    assign_square_uv(plane_mesh)
    plane = bpy.data.objects.new("SmokePlane", plane_mesh)
    scene.collection.objects.link(plane)


def area_3d(triangle):
    points = triangle["positions_m"]
    a = [points[1][i] - points[0][i] for i in range(3)]
    b = [points[2][i] - points[0][i] for i in range(3)]
    cross = (
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    )
    return math.sqrt(sum(component * component for component in cross)) / 2.0


def area_uv(triangle):
    uv = triangle["uv"]
    determinant = (uv[1][0] - uv[0][0]) * (uv[2][1] - uv[0][1]) - (
        uv[1][1] - uv[0][1]
    ) * (uv[2][0] - uv[0][0])
    return abs(determinant) / 2.0


def main():
    exporter = load_exporter()
    build_test_scene()

    scene = bpy.context.scene
    scene.unit_settings.system = "NONE"
    try:
        exporter.collect()
    except ValueError as error:
        assert "METRIC" in str(error), error
    else:
        raise AssertionError("Exporter must reject non-metric scene units")
    finally:
        scene.unit_settings.system = "METRIC"

    previous_argv = sys.argv
    try:
        with tempfile.TemporaryDirectory(prefix="dd3d-blender-uv-") as temp_dir:
            output = Path(temp_dir) / "uv_dump.json"
            sys.argv = [sys.argv[0], "--", "--out", str(output)]
            exporter.main()

            original_bytes = output.read_bytes()
            data = json.loads(original_bytes.decode("utf-8"))
            assert data["schema_version"] == 1, data

            by_object = {}
            for triangle in data["triangles"]:
                by_object.setdefault(triangle["object"], []).append(triangle)
                assert len(triangle["positions_m"]) == 3, triangle
                assert len(triangle["uv"]) == 3, triangle

            assert len(by_object.get("Cube", [])) == 12, by_object
            plane = by_object.get("SmokePlane", [])
            assert len(plane) == 2, by_object

            world_area = sum(area_3d(triangle) for triangle in plane)
            texture_area = sum(area_uv(triangle) for triangle in plane)
            td_at_128 = 128.0 * math.sqrt(texture_area / world_area)
            assert math.isclose(world_area, 1.0, abs_tol=1e-9), world_area
            assert math.isclose(texture_area, 1.0, abs_tol=1e-9), texture_area
            assert math.isclose(td_at_128, 128.0, abs_tol=1e-9), td_at_128

            try:
                exporter.main()
            except FileExistsError:
                pass
            else:
                raise AssertionError("Exporter must not overwrite an existing JSON dump")
            assert output.read_bytes() == original_bytes, "Existing JSON dump was changed"
    finally:
        sys.argv = previous_argv

    print(
        "BLENDER_UV_SMOKE_OK",
        f"version={bpy.app.version_string}",
        "schema=1",
        "cube_triangles=12",
        "plane_triangles=2",
        "plane_area_m2=1.0",
        "uv_area=1.0",
        "td_at_128=128px/m",
        "overwrite_rejected=true",
    )


if __name__ == "__main__":
    main()
