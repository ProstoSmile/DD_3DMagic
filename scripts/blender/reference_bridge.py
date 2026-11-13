"""Bounded DD reference proxy bridge. No user code, imports or Blender operators in jobs.

Run with factory startup. Build creates only a NEW staged scene. Inspect opens that
scene in a separate process and reports evaluated world geometry, not stored bounds.
"""
import argparse
import json
import math
from pathlib import Path
import sys

import bpy
from mathutils import Matrix, Vector
from bpy_extras.object_utils import world_to_camera_view


def on_d(path):
    path = Path(path).resolve()
    if path.drive.lower() not in ("d:", "\\\\?\\d:"):
        raise ValueError("PATH_OUTSIDE_D: " + str(path))
    return path


def new_path(root, name):
    path = on_d(root / name)
    if not path.is_relative_to(root) or path.exists():
        raise ValueError("Unsafe or existing output: " + str(path))
    return path


def geometry_report():
    components = []
    depsgraph = bpy.context.evaluated_depsgraph_get()
    for obj in sorted(bpy.context.scene.objects, key=lambda obj: obj.name):
        if obj.type != "MESH":
            continue
        evaluated = obj.evaluated_get(depsgraph)
        mesh = evaluated.to_mesh()
        try:
            mesh.calc_loop_triangles()
            vertices = [list(evaluated.matrix_world @ vertex.co) for vertex in mesh.vertices]
            components.append({
                "id": obj.get("dd_component_id", obj.name),
                "vertices": vertices,
                "faces": [list(poly.vertices) for poly in mesh.polygons],
                "triangles": [list(triangle.vertices) for triangle in mesh.loop_triangles],
                "bounds_min": [min(v[i] for v in vertices) for i in range(3)],
                "bounds_max": [max(v[i] for v in vertices) for i in range(3)],
            })
        finally:
            evaluated.to_mesh_clear()
    scene = bpy.context.scene
    unit_settings = {
        "system": scene.unit_settings.system,
        "scale_length": float(scene.unit_settings.scale_length),
        "length_unit": scene.unit_settings.length_unit,
    }
    units = scene.get("dd_units")
    if units == "meters" and (
        unit_settings["system"] != "METRIC"
        or abs(unit_settings["scale_length"] - 1.0) > 1e-12
        or unit_settings["length_unit"] != "METERS"
    ):
        raise ValueError("SAVE_VALIDATION_FAILED: meter GeometrySpec needs Blender Metric meters at scale_length 1.0")
    dependencies = []
    for image in bpy.data.images:
        if image.source == "FILE" and image.filepath and not image.packed_file:
            actual = on_d(bpy.path.abspath(image.filepath))
            if not actual.is_file():
                raise ValueError("Missing texture: " + str(actual))
            dependencies.append(str(actual))
    for library in bpy.data.libraries:
        actual = on_d(bpy.path.abspath(library.filepath, library=library.parent))
        if not actual.is_file():
            raise ValueError("Missing linked library: " + str(actual))
        dependencies.append(str(actual))
    return {
        "schema_version": 1,
        "blender_version": bpy.app.version_string,
        "asset_uid": scene.get("dd_asset_uid"),
        "revision": scene.get("dd_revision"),
        "components": components,
        "dependencies": dependencies,
        "units": units,
        "unit_settings": unit_settings,
    }


def build(job, root):
    scene = bpy.context.scene
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    scene["dd_asset_uid"] = job["asset_uid"]
    scene["dd_revision"] = job["revision"]
    scene["dd_units"] = job["units"]
    if job["units"] == "meters":
        scene.unit_settings.system = "METRIC"
        scene.unit_settings.scale_length = 1.0
        scene.unit_settings.length_unit = "METERS"
    else:
        # Legacy relative-unit assets remain explicitly unitless.
        scene.unit_settings.system = "NONE"
    for component in job["components"]:
        mesh = bpy.data.meshes.new(component["id"])
        polygons = component.get("faces") or component["triangles"]
        mesh.from_pydata(component["vertices"], [], polygons)
        mesh.update()
        obj = bpy.data.objects.new(component["id"], mesh)
        scene.collection.objects.link(obj)
        obj["dd_component_id"] = component["id"]
        material = bpy.data.materials.new(component["id"] + "_material")
        material.diffuse_color = component.get("color", [0.3, 0.35, 0.4, 1.0])
        obj.data.materials.append(material)
    bpy.context.preferences.filepaths.save_version = 0
    bpy.context.preferences.filepaths.temporary_directory = str(root)
    output = new_path(root, "scene.blend")
    bpy.ops.wm.save_as_mainfile(filepath=str(output), check_existing=False)
    return geometry_report()


def render_views(job, root):
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = 8
    scene.render.resolution_x = 640
    scene.render.resolution_y = 240
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGBA"
    scene.render.film_transparent = True
    scene.render.use_compositing = False
    scene.render.use_sequencer = False
    scene.world.color = (0.4, 0.4, 0.4)
    vertices = [v for c in job["components"] for v in c["vertices"]]
    low = Vector([min(v[i] for v in vertices) for i in range(3)])
    high = Vector([max(v[i] for v in vertices) for i in range(3)])
    center = (low + high) / 2
    radius = max((high - low).length, 1.0)
    lamp = bpy.data.lights.new("DD_key", "AREA")
    lamp.energy = 2000
    lamp.shape = "DISK"
    lamp.size = radius * 2
    light = bpy.data.objects.new("DD_key", lamp)
    scene.collection.objects.link(light)
    light.location = center + Vector((0, -radius, radius * 1.5))
    light.rotation_euler = (center - light.location).to_track_quat("-Z", "Y").to_euler()
    bases = {}
    for name in ("side", "top", "front", "three_quarter"):
        if name not in job["cameras"]:
            continue
        basis = Matrix([row[:3] for row in job["cameras"][name][:3]])
        bases[name] = basis
    # A single world-unit scale makes the three principal views directly comparable.
    scale = max(
        max(abs(point.x), abs(point.y) * 640 / 240) * 2
        for basis in bases.values()
        for point in (basis @ (Vector(v) - center) for v in vertices)
    ) * 1.12
    rendered = []
    for name, basis in bases.items():
        camera_data = bpy.data.cameras.new("DD_" + name)
        camera_data.type = "ORTHO"
        camera_data.sensor_fit = "HORIZONTAL"
        camera = bpy.data.objects.new("DD_" + name, camera_data)
        scene.collection.objects.link(camera)
        camera.location = center + basis[2] * radius * 3
        camera.rotation_euler = basis.transposed().to_euler()
        camera_data.ortho_scale = scale
        camera_data.clip_end = max(1000.0, radius * 6)
        bpy.context.view_layer.update()
        scene.camera = camera
        output = new_path(root, "blender_" + name + ".png")
        scene.render.filepath = str(output)
        bpy.ops.render.render(write_still=True)
        silhouette = compare_silhouette(job, camera, output, root, name)
        rendered.append({"view": name, "path": output.name,
                         "matrix_world": [list(row) for row in camera.matrix_world],
                         "ortho_scale": camera_data.ortho_scale,
                         "silhouette": silhouette})
    return rendered


def compare_silhouette(job, camera, output, root, name):
    """Rasterize expected source triangles and compare with actual Blender render alpha.

    A one-pixel boundary allowance accounts for center sampling versus Cycles AA;
    interior or displaced silhouette pixels beyond this allowance fail the gate.
    """
    width, height = 640, 240
    mask = bytearray(width * height)
    for component in job["components"]:
        vertices = [world_to_camera_view(bpy.context.scene, camera, Vector(v))
                    for v in component["vertices"]]
        points = [(v.x * width, v.y * height) for v in vertices]
        for face in component["triangles"]:
            a, b, c = [points[i] for i in face]
            determinant = ((b[1] - c[1]) * (a[0] - c[0])
                           + (c[0] - b[0]) * (a[1] - c[1]))
            if abs(determinant) < 1e-10:
                continue
            for y in range(max(0, math.floor(min(a[1], b[1], c[1]))),
                           min(height, math.ceil(max(a[1], b[1], c[1])))):
                for x in range(max(0, math.floor(min(a[0], b[0], c[0]))),
                               min(width, math.ceil(max(a[0], b[0], c[0])))):
                    u = ((b[1] - c[1]) * (x + .5 - c[0])
                         + (c[0] - b[0]) * (y + .5 - c[1])) / determinant
                    v = ((c[1] - a[1]) * (x + .5 - c[0])
                         + (a[0] - c[0]) * (y + .5 - c[1])) / determinant
                    if u >= 0 and v >= 0 and u + v <= 1:
                        mask[y * width + x] = 1
    loaded = bpy.data.images.load(str(output), check_existing=False)
    rgba = list(loaded.pixels)
    actual = bytearray(rgba[i * 4 + 3] >= .5 for i in range(width * height))
    bpy.data.images.remove(loaded)
    union = sum(bool(a or b) for a, b in zip(mask, actual))
    intersection = sum(bool(a and b) for a, b in zip(mask, actual))
    if not union:
        raise ValueError("SAVE_VALIDATION_FAILED: empty render " + name)

    def within_boundary(index, other):
        x, y = index % width, index // width
        return any(other[yy * width + xx]
                   for yy in range(max(0, y - 1), min(height, y + 2))
                   for xx in range(max(0, x - 1), min(width, x + 2)))

    outside = sum(1 for i, (a, b) in enumerate(zip(mask, actual))
                  if a != b and not within_boundary(i, actual if a else mask))
    if outside:
        raise ValueError("SAVE_VALIDATION_FAILED: silhouette " + name
                         + " has " + str(outside) + " pixels beyond 1px tolerance")
    image = bpy.data.images.new("DD_expected_" + name, width, height, alpha=True)
    pixels = [channel for value in mask for channel in (1.0, 1.0, 1.0, float(value))]
    image.pixels.foreach_set(pixels)
    image.filepath_raw = str(new_path(root, "expected_" + name + "_mask.png"))
    image.file_format = "PNG"
    image.save()
    bpy.data.images.remove(image)
    return {"status": "PASS", "iou": intersection / union,
            "boundary_tolerance_px": 1, "mismatch_beyond_tolerance": outside,
            "expected_pixels": sum(mask), "actual_pixels": sum(actual)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=["build", "inspect"])
    parser.add_argument("job")
    parser.add_argument("--renders", action="store_true")
    args = parser.parse_args(sys.argv[sys.argv.index("--") + 1:])
    job_path = on_d(args.job)
    root = job_path.parent
    if job_path.stat().st_size > 32 * 1024 * 1024:
        raise ValueError("Job exceeds safety limit")
    job = json.loads(job_path.read_text(encoding="utf-8"))
    if job["schema_version"] != 1 or not 0 < len(job["components"]) <= 256:
        raise ValueError("Invalid bounded bridge job")
    for component in job["components"]:
        if not component["vertices"] or len(component["vertices"]) > 100000:
            raise ValueError("Invalid mesh vertex count")
        if any(not math.isfinite(x) for v in component["vertices"] for x in v):
            raise ValueError("Non-finite vertex")
    result = build(job, root) if args.mode == "build" else geometry_report()
    if args.renders:
        result["renders"] = render_views(job, root)
    report = new_path(root, args.mode + "_report.json")
    with report.open("x", encoding="utf-8") as stream:
        json.dump(result, stream, indent=2, allow_nan=False)


if __name__ == "__main__":
    main()
