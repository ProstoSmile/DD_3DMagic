"""Build a staged, editable low-poly TOZ-34 game prop and pixel-art textures.

Run with:
  blender --background --factory-startup --python-exit-code 1 \
    --python scripts/blender/build_toz34_asset.py -- --output-dir <staging-dir>

This is an exploratory static game asset, not an engineering-accurate model.
It uses only the project's local material palette and writes new staged files.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
from array import array
from pathlib import Path

import bpy
import bmesh
from mathutils import Matrix, Vector


REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_OUTPUT = (
    REPO_ROOT / "demo_project" / ".assetlab" / "staging" / "toz34_model_v02"
)
STAGING_ROOT = (REPO_ROOT / "demo_project" / ".assetlab" / "staging").resolve()
MODEL_VERSION = "model_v02"
BLEND_FILENAME = "TOZ34_Static_Prop_v02.blend"
ATLAS_SIZE = 256
TD_PX_PER_M = 128.0
PAD_PX = 4.0

PALETTE = {
    "ink": (14, 20, 24),
    "steel_dark": (38, 48, 50),
    "steel_mid": (66, 77, 70),
    "earth_shadow": (104, 98, 72),
    "wood_light": (159, 129, 79),
    "warm_highlight": (204, 179, 119),
    "wood_dark": (101, 57, 42),
    "wood_mid": (155, 88, 52),
    "wood_spark": (230, 209, 153),
}

# The passport prints 1150 x 55 x 210 mm. Axis labels are not present beside
# that value, so this mapping is a provisional art-model assumption.
BOUNDS = {
    "xmin": -0.575,
    "xmax": 0.575,
    "ymin": -0.0275,
    "ymax": 0.0275,
    "zmin": 0.0,
    "zmax": 0.210,
}


def cli_args() -> argparse.Namespace:
    forwarded = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else []
    parser = argparse.ArgumentParser()
    parser.add_argument("--output-dir", type=Path, default=DEFAULT_OUTPUT)
    parser.add_argument(
        "--replace-existing",
        action="store_true",
        help="Replace only a draft output folder already under the project staging root.",
    )
    return parser.parse_args(forwarded)


def remove_startup_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for datablocks in (bpy.data.meshes, bpy.data.curves, bpy.data.cameras, bpy.data.lights):
        for datablock in list(datablocks):
            if datablock.users == 0:
                datablocks.remove(datablock)


def ensure_collection(name: str, parent: bpy.types.Collection) -> bpy.types.Collection:
    collection = bpy.data.collections.new(name)
    parent.children.link(collection)
    return collection


def move_to_collection(obj: bpy.types.Object, target: bpy.types.Collection) -> None:
    for collection in list(obj.users_collection):
        collection.objects.unlink(obj)
    target.objects.link(obj)


def rgb_float(color: tuple[int, int, int]) -> tuple[float, float, float, float]:
    return (color[0] / 255.0, color[1] / 255.0, color[2] / 255.0, 1.0)


def blank_canvas(color: tuple[int, int, int]) -> list[list[tuple[int, int, int]]]:
    return [[color for _ in range(ATLAS_SIZE)] for _ in range(ATLAS_SIZE)]


def pixel_rect(
    canvas: list[list[tuple[int, int, int]]],
    x0: int,
    y0: int,
    x1: int,
    y1: int,
    color: tuple[int, int, int],
) -> None:
    x0 = max(0, min(ATLAS_SIZE, x0))
    x1 = max(0, min(ATLAS_SIZE, x1))
    y0 = max(0, min(ATLAS_SIZE, y0))
    y1 = max(0, min(ATLAS_SIZE, y1))
    for y in range(y0, y1):
        canvas[y][x0:x1] = [color] * (x1 - x0)


def pixel_line(
    canvas: list[list[tuple[int, int, int]]],
    points: list[tuple[int, int]],
    color: tuple[int, int, int],
    thickness: int = 1,
) -> None:
    if len(points) < 2:
        return
    for (x0, y0), (x1, y1) in zip(points, points[1:]):
        dx = x1 - x0
        dy = y1 - y0
        steps = max(abs(dx), abs(dy), 1)
        for step in range(steps + 1):
            x = round(x0 + dx * step / steps)
            y = round(y0 + dy * step / steps)
            before = (thickness - 1) // 2
            after = thickness // 2 + 1
            pixel_rect(canvas, x - before, y - before, x + after, y + after, color)


def wood_canvas() -> list[list[tuple[int, int, int]]]:
    dark = PALETTE["wood_dark"]
    mid = PALETTE["wood_mid"]
    light = PALETTE["wood_light"]
    warm = PALETTE["warm_highlight"]
    spark = PALETTE["wood_spark"]
    shadow = PALETTE["earth_shadow"]
    canvas = blank_canvas(mid)

    # Side projection zone: 4..152 px wide by 4..31 px high (128 px/m).
    pixel_rect(canvas, 4, 4, 152, 31, mid)
    for y in (4, 29):
        pixel_line(canvas, [(4, y), (22, y - 1), (48, y), (86, y - 1), (120, y), (151, y - 1)], dark, 1)
    pixel_line(canvas, [(10, 8), (25, 7), (40, 8), (55, 7), (66, 8)], dark, 2)
    pixel_line(canvas, [(24, 10), (40, 9), (58, 10), (73, 9), (88, 10)], light, 1)
    pixel_line(canvas, [(5, 14), (20, 13), (36, 14), (48, 13), (60, 14)], shadow, 2)
    pixel_line(canvas, [(31, 16), (47, 15), (63, 16), (81, 15), (96, 16)], warm, 1)
    pixel_line(canvas, [(5, 20), (19, 19), (38, 20), (51, 19), (64, 20)], dark, 1)
    pixel_line(canvas, [(61, 22), (78, 21), (94, 22), (113, 21), (131, 22), (148, 21)], light, 1)
    pixel_line(canvas, [(17, 25), (34, 24), (49, 25), (70, 24)], warm, 1)
    pixel_line(canvas, [(93, 26), (108, 25), (121, 26), (143, 25)], dark, 1)

    # One deliberate knot cluster, built from stepped bands rather than noise.
    pixel_rect(canvas, 82, 11, 93, 21, dark)
    pixel_rect(canvas, 84, 12, 91, 20, mid)
    pixel_rect(canvas, 86, 13, 90, 18, shadow)
    pixel_rect(canvas, 87, 14, 89, 17, light)
    pixel_rect(canvas, 88, 14, 89, 15, spark)
    pixel_line(canvas, [(78, 12), (81, 11), (83, 12)], warm, 1)
    pixel_line(canvas, [(92, 20), (96, 21), (101, 20)], dark, 1)

    # Top projection zone for narrow surfaces; direction remains longitudinal.
    pixel_rect(canvas, 4, 40, 152, 49, dark)
    pixel_line(canvas, [(4, 42), (30, 41), (57, 42), (82, 41), (111, 42), (151, 41)], mid, 2)
    pixel_line(canvas, [(9, 46), (42, 45), (77, 46), (112, 45), (149, 46)], light, 1)

    # End-grain projection zone.
    pixel_rect(canvas, 170, 4, 180, 31, dark)
    pixel_rect(canvas, 172, 6, 178, 29, mid)
    pixel_line(canvas, [(173, 8), (176, 10), (173, 12), (176, 14), (173, 16)], light, 1)
    pixel_line(canvas, [(176, 18), (173, 20), (176, 22), (173, 24), (176, 26)], warm, 1)
    return canvas


def steel_canvas() -> list[list[tuple[int, int, int]]]:
    ink = PALETTE["ink"]
    dark = PALETTE["steel_dark"]
    mid = PALETTE["steel_mid"]
    earth = PALETTE["earth_shadow"]
    warm = PALETTE["warm_highlight"]
    canvas = blank_canvas(dark)

    # Side projection: large stepped planes and controlled edge highlights.
    pixel_rect(canvas, 4, 4, 152, 31, dark)
    pixel_line(canvas, [(4, 5), (30, 5), (54, 6), (88, 5), (115, 6), (151, 5)], mid, 2)
    pixel_line(canvas, [(8, 10), (39, 10), (73, 11), (103, 10), (145, 11)], ink, 2)
    pixel_line(canvas, [(12, 15), (42, 15), (64, 16), (97, 15)], mid, 1)
    pixel_line(canvas, [(65, 20), (85, 20), (96, 19), (114, 20), (140, 19)], ink, 2)
    pixel_line(canvas, [(5, 25), (29, 24), (54, 25), (82, 24), (109, 25), (151, 24)], mid, 1)
    pixel_rect(canvas, 73, 8, 99, 17, ink)
    pixel_rect(canvas, 75, 9, 96, 15, dark)
    pixel_line(canvas, [(77, 10), (88, 10), (94, 11)], mid, 1)
    pixel_line(canvas, [(8, 7), (24, 7), (32, 8)], warm, 1)
    pixel_line(canvas, [(118, 6), (136, 6), (148, 7)], earth, 1)

    # Top projection: narrow longitudinal rib and planar glints.
    pixel_rect(canvas, 4, 40, 152, 49, ink)
    pixel_line(canvas, [(4, 42), (33, 42), (65, 43), (97, 42), (129, 43), (151, 42)], mid, 2)
    pixel_line(canvas, [(15, 46), (54, 45), (92, 46), (134, 45)], earth, 1)

    # Front/end projection: dark inset centers and a stepped rim cue.
    pixel_rect(canvas, 170, 4, 180, 31, ink)
    pixel_rect(canvas, 171, 6, 179, 29, mid)
    pixel_rect(canvas, 172, 7, 178, 28, dark)
    pixel_line(canvas, [(173, 9), (177, 9), (177, 12), (173, 12)], ink, 1)
    pixel_line(canvas, [(173, 20), (177, 20), (177, 23), (173, 23)], ink, 1)
    return canvas


def pad_canvas() -> list[list[tuple[int, int, int]]]:
    ink = PALETTE["ink"]
    dark = PALETTE["steel_dark"]
    mid = PALETTE["steel_mid"]
    canvas = blank_canvas(ink)
    pixel_rect(canvas, 4, 4, 152, 31, dark)
    for x in range(8, 150, 8):
        pixel_line(canvas, [(x, 5), (x + 4, 9), (x + 1, 13)], mid, 1)
        pixel_line(canvas, [(x + 3, 18), (x, 22), (x + 4, 27)], ink, 1)
    pixel_rect(canvas, 4, 40, 152, 49, ink)
    pixel_line(canvas, [(4, 43), (38, 42), (77, 43), (113, 42), (151, 43)], dark, 2)
    pixel_rect(canvas, 170, 4, 180, 31, ink)
    pixel_line(canvas, [(171, 6), (178, 6), (178, 28), (171, 28), (171, 6)], mid, 1)
    return canvas


def save_pixel_image(
    name: str,
    canvas: list[list[tuple[int, int, int]]],
    output_path: Path,
) -> bpy.types.Image:
    image = bpy.data.images.new(name, width=ATLAS_SIZE, height=ATLAS_SIZE, alpha=False)
    image.colorspace_settings.name = "sRGB"
    pixels = array("f")
    # Blender image buffers are row-major from the lower-left origin.
    for y in range(ATLAS_SIZE):
        for x in range(ATLAS_SIZE):
            pixels.extend(rgb_float(canvas[y][x]))
    image.pixels.foreach_set(pixels)
    image.update()
    image.filepath_raw = str(output_path)
    image.file_format = "PNG"
    image.save()
    image.pack()
    image.use_fake_user = True
    return image


def make_material(
    name: str,
    image: bpy.types.Image,
    metallic: float,
    roughness: float,
    role: str,
) -> bpy.types.Material:
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    material.diffuse_color = rgb_float(PALETTE["wood_mid"] if role == "wood" else PALETTE["steel_dark"])
    material["material_role"] = role
    nodes = material.node_tree.nodes
    shader = nodes.get("Principled BSDF")
    shader.inputs["Metallic"].default_value = metallic
    shader.inputs["Roughness"].default_value = roughness
    texture = nodes.new("ShaderNodeTexImage")
    texture.name = f"{name} | nearest pixel atlas"
    texture.image = image
    texture.interpolation = "Closest"
    texture.extension = "CLIP"
    material.node_tree.links.new(texture.outputs["Color"], shader.inputs["Base Color"])
    material["atlas_resolution"] = ATLAS_SIZE
    material["pixel_sampling"] = "Closest"
    return material


def recalc_normals(obj: bpy.types.Object) -> None:
    if obj.type != "MESH":
        return
    bm = bmesh.new()
    bm.from_mesh(obj.data)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    bm.to_mesh(obj.data)
    bm.free()
    obj.data.update()


def apply_bevel(obj: bpy.types.Object, width: float, segments: int = 1) -> None:
    if width <= 0.0 or obj.type != "MESH":
        return
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    modifier = obj.modifiers.new("Single-segment low-poly chamfers", "BEVEL")
    modifier.width = width
    modifier.segments = segments
    modifier.limit_method = "ANGLE"
    modifier.angle_limit = math.radians(18.0)
    bpy.ops.object.modifier_apply(modifier=modifier.name)
    obj.select_set(False)
    recalc_normals(obj)


def set_uv_map(obj: bpy.types.Object) -> None:
    mesh = obj.data
    mesh.update()
    uv_layer = mesh.uv_layers.get("UV_Main") or mesh.uv_layers.new(name="UV_Main")
    uv_layer.active_render = True
    for polygon in mesh.polygons:
        normal = polygon.normal
        dominant = max(range(3), key=lambda axis: abs(normal[axis]))
        for loop_index in polygon.loop_indices:
            vertex_index = mesh.loops[loop_index].vertex_index
            co = mesh.vertices[vertex_index].co
            if dominant == 1:  # side: X/Z
                px = PAD_PX + (co.x - BOUNDS["xmin"]) * TD_PX_PER_M
                py = PAD_PX + (co.z - BOUNDS["zmin"]) * TD_PX_PER_M
            elif dominant == 2:  # top/bottom: X/Y
                px = PAD_PX + (co.x - BOUNDS["xmin"]) * TD_PX_PER_M
                py = 40.0 + PAD_PX + (co.y - BOUNDS["ymin"]) * TD_PX_PER_M
            else:  # front/back: Y/Z
                px = 170.0 + (co.y - BOUNDS["ymin"]) * TD_PX_PER_M
                py = PAD_PX + (co.z - BOUNDS["zmin"]) * TD_PX_PER_M
            uv_layer.data[loop_index].uv = (px / ATLAS_SIZE, py / ATLAS_SIZE)
    mesh.update()


def finish_object(
    obj: bpy.types.Object,
    name: str,
    material: bpy.types.Material,
    collection: bpy.types.Collection,
    component_uid: str,
    bevel: float = 0.0,
) -> bpy.types.Object:
    obj.name = name
    move_to_collection(obj, collection)
    obj.data.materials.append(material)
    obj["component_uid"] = component_uid
    obj["editable_artist_part"] = True
    if bevel > 0:
        apply_bevel(obj, bevel)
    if obj.type == "MESH":
        recalc_normals(obj)
        set_uv_map(obj)
    return obj


def profile_prism(
    name: str,
    profile: list[tuple[float, float]],
    half_depth: float,
    center_y: float,
    material: bpy.types.Material,
    collection: bpy.types.Collection,
    component_uid: str,
    bevel: float = 0.0,
) -> bpy.types.Object:
    count = len(profile)
    vertices = [(x, center_y - half_depth, z) for x, z in profile]
    vertices += [(x, center_y + half_depth, z) for x, z in profile]
    faces: list[tuple[int, ...]] = [tuple(range(count)), tuple(range(count, 2 * count))[::-1]]
    for index in range(count):
        next_index = (index + 1) % count
        faces.append((index, next_index, next_index + count, index + count))
    mesh = bpy.data.meshes.new(f"{name}_Mesh")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    collection.objects.link(obj)
    return finish_object(obj, name, material, collection, component_uid, bevel)


def box_object(
    name: str,
    center: tuple[float, float, float],
    size: tuple[float, float, float],
    material: bpy.types.Material,
    collection: bpy.types.Collection,
    component_uid: str,
    bevel: float = 0.0,
) -> bpy.types.Object:
    bpy.ops.mesh.primitive_cube_add(size=1.0, location=center)
    obj = bpy.context.object
    obj.dimensions = size
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    return finish_object(obj, name, material, collection, component_uid, bevel)


def cylinder_object(
    name: str,
    radius: float,
    depth: float,
    center: tuple[float, float, float],
    axis: str,
    material: bpy.types.Material,
    collection: bpy.types.Collection,
    component_uid: str,
    vertices: int = 12,
    bevel: float = 0.0,
) -> bpy.types.Object:
    rotation = (0.0, 0.0, 0.0)
    if axis == "X":
        rotation = (0.0, math.pi / 2.0, 0.0)
    elif axis == "Y":
        rotation = (math.pi / 2.0, 0.0, 0.0)
    bpy.ops.mesh.primitive_cylinder_add(
        vertices=vertices,
        radius=radius,
        depth=depth,
        end_fill_type="NGON",
        location=center,
        rotation=rotation,
    )
    obj = bpy.context.object
    bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
    return finish_object(obj, name, material, collection, component_uid, bevel)


def loft_object(
    name: str,
    stations: list[tuple[float, float, float, float, float]],
    material: bpy.types.Material,
    collection: bpy.types.Collection,
    component_uid: str,
    segments: int = 8,
) -> bpy.types.Object:
    vertices: list[tuple[float, float, float]] = []
    for x, cy, cz, ry, rz in stations:
        for side in range(segments):
            angle = 2.0 * math.pi * side / segments + math.pi / 8.0
            vertices.append((x, cy + ry * math.cos(angle), cz + rz * math.sin(angle)))
    faces: list[tuple[int, ...]] = [tuple(range(segments))[::-1]]
    faces.append(tuple(range((len(stations) - 1) * segments, len(stations) * segments)))
    for ring in range(len(stations) - 1):
        for side in range(segments):
            nxt = (side + 1) % segments
            a = ring * segments + side
            b = ring * segments + nxt
            c = (ring + 1) * segments + nxt
            d = (ring + 1) * segments + side
            faces.append((a, b, c, d))
    mesh = bpy.data.meshes.new(f"{name}_Mesh")
    mesh.from_pydata(vertices, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    collection.objects.link(obj)
    return finish_object(obj, name, material, collection, component_uid, 0.0015)


def curve_tube(
    name: str,
    points: list[tuple[float, float, float]],
    radius: float,
    material: bpy.types.Material,
    collection: bpy.types.Collection,
    component_uid: str,
) -> bpy.types.Object:
    curve = bpy.data.curves.new(f"{name}_Curve", type="CURVE")
    curve.dimensions = "3D"
    curve.resolution_u = 1
    curve.bevel_resolution = 0
    curve.bevel_depth = radius
    spline = curve.splines.new(type="POLY")
    spline.points.add(len(points) - 1)
    for point, co in zip(spline.points, points):
        point.co = (co[0], co[1], co[2], 1.0)
    obj = bpy.data.objects.new(name, curve)
    collection.objects.link(obj)
    obj.data.materials.append(material)
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.convert(target="MESH")
    obj = bpy.context.object
    obj.name = name
    obj["component_uid"] = component_uid
    obj["editable_artist_part"] = True
    set_uv_map(obj)
    return obj


def build_geometry(
    collection: bpy.types.Collection,
    wood: bpy.types.Material,
    steel: bpy.types.Material,
    pad: bpy.types.Material,
) -> list[bpy.types.Object]:
    created: list[bpy.types.Object] = []

    # Walnut stock with a conservative pistol-grip curve and broad cheek.
    stock_profile = [
        (-0.145, 0.184),
        (-0.203, 0.179),
        (-0.315, 0.171),
        (-0.448, 0.158),
        (-0.558, 0.146),
        (-0.571, 0.137),
        (-0.571, 0.018),
        (-0.548, 0.010),
        (-0.454, 0.021),
        (-0.378, 0.037),
        (-0.321, 0.044),
        (-0.287, 0.058),
        (-0.277, 0.078),
        (-0.292, 0.101),
        (-0.276, 0.132),
        (-0.235, 0.158),
        (-0.178, 0.164),
    ]
    created.append(
        profile_prism(
            "TOZ34_Stock_Wood",
            stock_profile,
            0.021,
            0.0,
            wood,
            collection,
            "stock_main",
            bevel=0.004,
        )
    )

    # Compact angled fore-end, kept separate from the barrels for editing.
    created.append(
        loft_object(
            "TOZ34_Foreend_Wood",
            [
                (-0.170, 0.0, 0.132, 0.012, 0.012),
                (-0.132, 0.0, 0.132, 0.021, 0.020),
                (-0.045, 0.0, 0.132, 0.022, 0.020),
                (0.194, 0.0, 0.132, 0.020, 0.019),
                (0.318, 0.0, 0.133, 0.016, 0.016),
            ],
            wood,
            collection,
            "foreend_main",
        )
    )

    # Dark butt plate is a visual placeholder; the exact base-model fitting is unverified.
    butt_profile = [(-0.574, 0.139), (-0.574, 0.016), (-0.570, 0.012), (-0.570, 0.143)]
    created.append(
        profile_prism(
            "TOZ34_Buttplate_Dark",
            butt_profile,
            0.0225,
            0.0,
            pad,
            collection,
            "buttplate_placeholder",
            bevel=0.001,
        )
    )

    # Over-under barrel pair; each remains an independent low-poly mesh.
    breech_x = BOUNDS["xmax"] - 0.711
    barrel_length = BOUNDS["xmax"] - breech_x
    barrel_centers = (0.165, 0.188)
    for index, z in enumerate(barrel_centers, start=1):
        created.append(
            cylinder_object(
                f"TOZ34_Barrel_{index:02d}_Steel",
                radius=0.0115,
                depth=barrel_length,
                center=((breech_x + BOUNDS["xmax"]) / 2.0, 0.0, z),
                axis="X",
                material=steel,
                collection=collection,
                component_uid=f"barrel_{index:02d}",
                vertices=12,
                bevel=0.0007,
            )
        )
        created.append(
            cylinder_object(
                f"TOZ34_Muzzle_Rim_{index:02d}",
                radius=0.0122,
                depth=0.0016,
                center=(BOUNDS["xmax"] - 0.0008, 0.0, z),
                axis="X",
                material=steel,
                collection=collection,
                component_uid=f"muzzle_rim_{index:02d}",
                vertices=12,
                bevel=0.0003,
            )
        )
        created.append(
            cylinder_object(
                f"TOZ34_Muzzle_Inset_{index:02d}",
                radius=0.0078,
                depth=0.0005,
                center=(BOUNDS["xmax"] - 0.00025, 0.0, z),
                axis="X",
                material=steel,
                collection=collection,
                component_uid=f"muzzle_inset_{index:02d}",
                vertices=12,
            )
        )

    # Narrow top rib and bead; it carries the recognizable long upper line.
    created.append(
        box_object(
            "TOZ34_Sight_Rib_Steel",
            (0.211, 0.0, 0.2015),
            (0.690, 0.0055, 0.0025),
            steel,
            collection,
            "sight_rib",
            bevel=0.0007,
        )
    )
    created.append(
        cylinder_object(
            "TOZ34_Front_Bead",
            radius=0.0027,
            depth=0.003,
            center=(0.548, 0.0, 0.2085),
            axis="Z",
            material=steel,
            collection=collection,
            component_uid="front_sight_bead",
            vertices=8,
        )
    )

    # Simplified base-action silhouette, bilateral plates and a top lever.
    receiver_profile = [
        (-0.197, 0.151),
        (-0.178, 0.181),
        (-0.135, 0.191),
        (-0.075, 0.189),
        (-0.042, 0.177),
        (-0.040, 0.145),
        (-0.075, 0.134),
        (-0.151, 0.137),
    ]
    created.append(
        profile_prism(
            "TOZ34_Receiver_Steel",
            receiver_profile,
            0.026,
            0.0,
            steel,
            collection,
            "receiver_body",
            bevel=0.002,
        )
    )
    plate_profile = [
        (-0.178, 0.158),
        (-0.157, 0.179),
        (-0.096, 0.181),
        (-0.062, 0.172),
        (-0.061, 0.149),
        (-0.091, 0.143),
        (-0.151, 0.145),
    ]
    for side, y in (("Near", -0.0263), ("Far", 0.0263)):
        created.append(
            profile_prism(
                f"TOZ34_Receiver_Sideplate_{side}",
                plate_profile,
                0.0012,
                y,
                steel,
                collection,
                f"receiver_sideplate_{side.lower()}",
                bevel=0.0008,
            )
        )

    created.append(
        box_object(
            "TOZ34_TopLever_Steel",
            (-0.123, 0.0, 0.192),
            (0.055, 0.008, 0.003),
            steel,
            collection,
            "top_lever",
            bevel=0.001,
        )
    )

    # External trigger bow and two simple trigger silhouettes.
    for side, y in (("Near", -0.0255), ("Far", 0.0255)):
        created.append(
            curve_tube(
                f"TOZ34_TriggerGuard_{side}",
                [
                (-0.159, y, 0.133),
                (-0.148, y, 0.104),
                (-0.125, y, 0.080),
                (-0.092, y, 0.073),
                (-0.053, y, 0.077),
                (-0.022, y, 0.095),
                (-0.018, y, 0.127),
                ],
                0.0018,
                steel,
                collection,
                f"trigger_guard_{side.lower()}",
            )
        )
    for index, x in enumerate((-0.118, -0.078), start=1):
        created.append(
            profile_prism(
                f"TOZ34_Trigger_{index:02d}",
                [
                    (x - 0.006, 0.109),
                    (x + 0.003, 0.108),
                    (x + 0.008, 0.083),
                    (x + 0.002, 0.080),
                ],
                0.0018,
                0.0,
                steel,
                collection,
                f"trigger_{index:02d}",
                bevel=0.0005,
            )
        )

    # Two low-profile fore-end fasteners on each side.
    for side, y in (("Near", -0.0265), ("Far", 0.0265)):
        for index, x in enumerate((0.045, 0.226), start=1):
            created.append(
                cylinder_object(
                    f"TOZ34_Foreend_Pin_{side}_{index}",
                    radius=0.0032,
                    depth=0.001,
                    center=(x, y, 0.137),
                    axis="Y",
                    material=steel,
                    collection=collection,
                    component_uid=f"foreend_pin_{side.lower()}_{index}",
                    vertices=8,
                )
            )

    # A modest extractor/hinge block at the action face, without internal detail.
    created.append(
        box_object(
            "TOZ34_Breech_Block_Steel",
            (-0.142, 0.0, 0.172),
            (0.024, 0.050, 0.034),
            steel,
            collection,
            "breech_exterior",
            bevel=0.0015,
        )
    )

    return created


def create_look_at_camera(
    name: str,
    location: tuple[float, float, float],
    target: tuple[float, float, float],
    ortho_scale: float,
    resolution: tuple[int, int],
    collection: bpy.types.Collection,
    world_up: tuple[float, float, float] = (0.0, 0.0, 1.0),
) -> bpy.types.Object:
    camera_data = bpy.data.cameras.new(name)
    camera = bpy.data.objects.new(name, camera_data)
    collection.objects.link(camera)
    camera.location = Vector(location)
    direction = (Vector(target) - camera.location).normalized()
    up_hint = Vector(world_up).normalized()
    right = direction.cross(up_hint).normalized()
    up = right.cross(direction).normalized()
    basis = Matrix((right, up, -direction)).transposed()
    camera.rotation_euler = basis.to_euler()
    camera_data.type = "ORTHO"
    camera_data.ortho_scale = ortho_scale
    camera_data.clip_start = 0.001
    camera_data.clip_end = 100.0
    camera["reference_camera"] = name
    camera["world_units_per_pixel_x"] = ortho_scale / resolution[0]
    return camera


def add_area_light(
    name: str,
    location: tuple[float, float, float],
    target: tuple[float, float, float],
    power: float,
    size: float,
    collection: bpy.types.Collection,
) -> bpy.types.Object:
    light_data = bpy.data.lights.new(name, type="AREA")
    light_data.energy = power
    light_data.shape = "DISK"
    light_data.size = size
    light = bpy.data.objects.new(name, light_data)
    collection.objects.link(light)
    light.location = location
    direction = Vector(target) - light.location
    light.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()
    return light


def configure_scene(
    scene: bpy.types.Scene,
    camera_collection: bpy.types.Collection,
    light_collection: bpy.types.Collection,
) -> dict[str, bpy.types.Object]:
    scene.render.engine = "BLENDER_EEVEE_NEXT"
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGB"
    scene.render.resolution_percentage = 100
    scene.render.film_transparent = False
    scene.render.use_file_extension = True
    scene.world.use_nodes = True
    background = scene.world.node_tree.nodes.get("Background")
    background.inputs["Color"].default_value = (0.32, 0.34, 0.35, 1.0)
    background.inputs["Strength"].default_value = 0.16
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "Medium High Contrast"
    scene.view_settings.exposure = -1.5
    scene.view_settings.gamma = 1.0
    scene.render.resolution_x = 1600
    scene.render.resolution_y = 1000
    scene.render.resolution_percentage = 100
    scene.camera = None

    center = (0.0, 0.0, 0.105)
    cameras = {
        "front": create_look_at_camera(
            "CAM_TOZ34_FRONT",
            (3.0, 0.0, 0.105),
            center,
            0.176,
            (256, 320),
            camera_collection,
        ),
        "side": create_look_at_camera(
            "CAM_TOZ34_SIDE_FAR",
            (0.0, 3.0, 0.105),
            center,
            1.240,
            (1800, 340),
            camera_collection,
        ),
        "top": create_look_at_camera(
            "CAM_TOZ34_TOP",
            (0.0, 0.0, 3.0),
            center,
            1.240,
            (1800, 140),
            camera_collection,
            world_up=(0.0, -1.0, 0.0),
        ),
        "three_quarter": create_look_at_camera(
            "CAM_TOZ34_3Q_FAR_FRONT",
            (1.7, 1.55, 0.72),
            center,
            1.18,
            (1600, 900),
            camera_collection,
        ),
    }
    add_area_light("LIGHT_Key", (0.10, -0.85, 1.35), center, 60.0, 1.0, light_collection)
    add_area_light("LIGHT_Fill", (-0.60, 0.75, 0.70), center, 25.0, 1.1, light_collection)
    add_area_light("LIGHT_Rim", (0.85, 0.45, 1.10), center, 35.0, 0.8, light_collection)
    return cameras


def render_camera(
    scene: bpy.types.Scene,
    camera: bpy.types.Object,
    output_path: Path,
    resolution: tuple[int, int],
) -> None:
    scene.camera = camera
    scene.render.resolution_x = resolution[0]
    scene.render.resolution_y = resolution[1]
    scene.render.filepath = str(output_path)
    bpy.ops.render.render(write_still=True)


def object_triangle_count(obj: bpy.types.Object) -> int:
    if obj.type != "MESH":
        return 0
    mesh = obj.data
    mesh.calc_loop_triangles()
    return len(mesh.loop_triangles)


def sha256(path: Path) -> str | None:
    if not path.is_file():
        return None
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def main() -> None:
    args = cli_args()
    output_dir = args.output_dir.resolve()
    try:
        output_dir.relative_to(STAGING_ROOT)
    except ValueError as error:
        raise ValueError(f"Output directory must stay under {STAGING_ROOT}") from error
    if output_dir.exists() and any(output_dir.iterdir()) and not args.replace_existing:
        raise FileExistsError(
            f"Refusing to replace existing staged output {output_dir}; "
            "use --replace-existing only for a draft you own."
        )
    output_dir.mkdir(parents=True, exist_ok=True)
    texture_dir = output_dir / "textures"
    render_dir = output_dir / "renders"
    texture_dir.mkdir(parents=True, exist_ok=True)
    render_dir.mkdir(parents=True, exist_ok=True)

    remove_startup_scene()
    scene = bpy.context.scene
    scene.name = "TOZ34_Static_Prop_v02"
    scene.unit_settings.system = "METRIC"
    scene.unit_settings.scale_length = 1.0
    asset_collection = ensure_collection("TOZ34 | Editable Asset", scene.collection)
    camera_collection = ensure_collection("Review Cameras", scene.collection)
    light_collection = ensure_collection("Studio Lights", scene.collection)

    wood_image = save_pixel_image(
        "T_toz34_wood_basecolor_256",
        wood_canvas(),
        texture_dir / "T_toz34_wood_basecolor_256.png",
    )
    steel_image = save_pixel_image(
        "T_toz34_metal_basecolor_256",
        steel_canvas(),
        texture_dir / "T_toz34_metal_basecolor_256.png",
    )
    pad_image = save_pixel_image(
        "T_toz34_buttplate_basecolor_256",
        pad_canvas(),
        texture_dir / "T_toz34_buttplate_basecolor_256.png",
    )
    wood = make_material("M_toz34_wood", wood_image, 0.0, 0.82, "wood")
    steel = make_material("M_toz34_steel", steel_image, 0.16, 0.78, "metal")
    pad = make_material("M_toz34_buttplate_placeholder", pad_image, 0.0, 0.9, "buttplate")

    objects = build_geometry(asset_collection, wood, steel, pad)
    for obj in objects:
        obj["reference_version"] = "toz34_reference_v03"
        obj["technical_reference_status"] = "NEEDS_EVIDENCE"
    cameras = configure_scene(scene, camera_collection, light_collection)

    scene["asset_id"] = "toz34"
    scene["asset_version"] = MODEL_VERSION
    scene["asset_scope"] = "static low-poly stylized game prop"
    scene["technical_reference_status"] = "NEEDS_EVIDENCE"
    scene["uv_map"] = "UV_Main"
    scene["uv_method"] = "world-axis orthographic projections; mirrored and stacked overlaps are intentional"
    scene["uv_target_td_px_per_m"] = TD_PX_PER_M
    scene["texture_style"] = "hand-authored pixel clusters, project palette, nearest sampling"
    scene["preservation"] = "new scene written only under .assetlab/staging; no source assets modified"

    view_specs = {
        "front": ((256, 320), "+X muzzle end; orthographic"),
        "side": ((1800, 340), "+Y far side; orthographic"),
        "top": ((1800, 140), "+Z; orthographic"),
        "three_quarter": ((1600, 900), "+X/+Y from above; orthographic"),
    }
    for view_name, (resolution, _description) in view_specs.items():
        render_camera(scene, cameras[view_name], render_dir / f"model_{view_name}.png", resolution)

    scene.camera = cameras["three_quarter"]
    scene.render.resolution_x = 1600
    scene.render.resolution_y = 900
    scene.render.filepath = str(render_dir / "model_three_quarter.png")
    blend_path = output_dir / BLEND_FILENAME
    bpy.ops.wm.save_as_mainfile(filepath=str(blend_path))

    mesh_objects = [obj for obj in objects if obj.type == "MESH"]
    bounds_min = [float("inf"), float("inf"), float("inf")]
    bounds_max = [float("-inf"), float("-inf"), float("-inf")]
    for obj in mesh_objects:
        for corner in obj.bound_box:
            world = obj.matrix_world @ Vector(corner)
            for axis in range(3):
                bounds_min[axis] = min(bounds_min[axis], world[axis])
                bounds_max[axis] = max(bounds_max[axis], world[axis])

    report = {
        "asset": "toz34",
        "version": MODEL_VERSION,
        "blend": blend_path.name,
        "blend_sha256": sha256(blend_path),
        "reference_version": "toz34_reference_v03",
        "technical_reference": "NEEDS_EVIDENCE",
        "units": "meters",
        "assumed_bounds_m": {"min": bounds_min, "max": bounds_max},
        "target_td_px_per_m": TD_PX_PER_M,
        "uv_map": "UV_Main",
        "uv_method": "axis-projected with intentional material-wise surface overlap",
        "texture_style": "hand-authored pixel clusters; project palette; closest sampling",
        "textures": [
            {"file": f"textures/{p.name}", "sha256": sha256(p)}
            for p in sorted(texture_dir.glob("*.png"))
        ],
        "renders": {name: f"renders/model_{name}.png" for name in view_specs},
        "mesh_object_count": len(mesh_objects),
        "triangle_count": sum(object_triangle_count(obj) for obj in mesh_objects),
        "components": [obj.name for obj in objects],
        "limits": [
            "technical front/top/side/3-4 reference is still incomplete",
            "overall dimension axes are provisional from the passport value order",
            "buttplate fitting is a visual placeholder",
            "UV projection overlap is intentional and still needs UV diagnostics",
        ],
    }
    (output_dir / "build_report.json").write_text(
        json.dumps(report, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    readme = f"""# TOZ-34 low-poly game prop — {MODEL_VERSION}

This is an editable static prop candidate built with Blender CLI. It is stored in staging and does not replace a source or approved asset.

## Contents

- `{BLEND_FILENAME}` — Blender scene with editable semantic parts, packed texture images, cameras and lights.
- `textures/` — three hand-authored pixel-art base-color atlases: wood, steel and a dark buttplate placeholder. All use the current demo-project palette and nearest texture sampling.
- `renders/` — candidate model renders from front/muzzle, side, top and orthographic 3/4 cameras. These are views of the model, not technical-reference images.
- `build_report.json` — object count, triangle count, bounds and output hashes.

## Current profile

- 24 mesh objects; triangle count and extents are in the report.
- Metric scene units, metre scale 1.0.
- `UV_Main` is generated by fixed world-axis projection at 128 px/m into separate 256×256 material atlases. Opposite-side overlaps are intentional; there is no claim of reviewed packing or a finished production UV layout.
- Wood uses stepped, directional grain clusters; steel uses sparse cool ramps and planar highlights. The texture pixels contain no CRT effect or baked lighting.

## Acceptance status and limits

`NEEDS_EVIDENCE`: reference v03 does not provide a verified same-specimen front/top/side/3/4 set. The passport's overall dimensions are printed as 1150×55×210 mm without adjacent axis labels; the model's axis assignment is provisional. This blockout is for visual iteration only. The dark buttplate is a placeholder and the exact base-model junctions have not been checked against a specimen.

The model has not passed independent visual/technical review. Review the staged `.blend`, inspect seams and UVs, and compare against the completed technical reference before promoting it.
"""
    (output_dir / "README.md").write_text(readme, encoding="utf-8")
    print(json.dumps(report, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
