"""Real stdio protocol regression: run after the reference CLI fixture is created."""
import json
import base64
import os
from pathlib import Path
import queue
import shutil
import subprocess
import threading
import uuid

root = Path(__file__).resolve().parents[1]
environment = dict(os.environ, DD_MAGIC_WORKSPACE_ROOT=str(root),
                   TEMP=str(root / ".work/temp"), TMP=str(root / ".work/temp"))
environment.pop("DD_MAGIC_PROJECT_ROOT", None)
process = subprocess.Popen([str(root / "target/debug/dd-3dmagic-mcp.exe")],
                           stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                           stderr=subprocess.PIPE, text=True, env=environment,
                           cwd=root, creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0))
messages = queue.Queue()


def read():
    for line in process.stdout:
        messages.put(json.loads(line))


threading.Thread(target=read, daemon=True).start()


def call(identifier, method, params):
    process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": identifier,
                                   "method": method, "params": params}) + "\n")
    process.stdin.flush()
    while True:
        result = messages.get(timeout=30)
        if result.get("id") == identifier:
            assert "error" not in result, result
            return result["result"]


def tool(identifier, name, arguments):
    result = call(identifier, "tools/call", {"name": name, "arguments": arguments})
    assert not result.get("isError"), result
    return json.loads(result["content"][0]["text"])


try:
    call(1, "initialize", {"protocolVersion": "2024-11-05", "capabilities": {},
                           "clientInfo": {"name": "dd-reference-smoke", "version": "1"}})
    process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
    process.stdin.flush()
    tools = call(2, "tools/list", {})["tools"]
    names = {item["name"] for item in tools}
    assert {"list_projects", "list_assets", "resolve_asset_context", "format_asset_names",
            "analyze_uv", "stage_pixel_texture", "reference_operation", "reference_frame"} <= names, names
    projects = tool(3, "list_projects", {})
    assert projects["ok"] and projects["result"], projects
    project_id = projects["result"][0]["project_id"]
    project_root = Path(projects["result"][0]["path"])
    frame_prefix = ".assetlab/staging/mcp_reference_frame_smoke_" + uuid.uuid4().hex
    frame_directory = project_root / frame_prefix / "invalid"
    frame_directory.mkdir(parents=True)

    checkpoint_dir = ".assetlab/staging/shotgun_line_blender_v3_bundled"
    checkpoint_path = project_root / checkpoint_dir / "blender_side.png"
    frame = call(30, "tools/call", {"name": "reference_frame", "arguments": {
        "project_id": project_id, "stage_relative": checkpoint_dir, "view": "side"}})
    assert not frame.get("isError"), frame
    assert [item["type"] for item in frame["content"]] == ["text", "image"], frame
    frame_metadata = json.loads(frame["content"][0]["text"])
    assert frame_metadata["ok"] and frame_metadata["result"]["filename"] == "blender_side.png", frame
    assert frame["content"][1]["mimeType"] == "image/png", frame
    assert frame["content"][1]["data"] == base64.b64encode(checkpoint_path.read_bytes()).decode("ascii")
    rejected_frame_path = call(31, "tools/call", {"name": "reference_frame", "arguments": {
        "project_id": project_id, "stage_relative": "../assets", "view": "side"}})
    assert rejected_frame_path.get("isError"), rejected_frame_path
    (frame_directory / "blender_side.png").write_bytes(b"not a png")
    rejected_frame_signature = call(32, "tools/call", {"name": "reference_frame", "arguments": {
        "project_id": project_id, "stage_relative": frame_prefix + "/invalid", "view": "side"}})
    assert rejected_frame_signature.get("isError"), rejected_frame_signature
    drawing_prefix = ".assetlab/staging/mcp_drawing_smoke_" + uuid.uuid4().hex

    def drawing(identifier, **request):
        return call(identifier, "tools/call", {
            "name": "reference_drawing",
            "arguments": {"project_id": project_id, "request": request},
        })

    blank_path = drawing_prefix + "/blank.json"
    ink_path = drawing_prefix + "/ink.json"
    blank = drawing(40, operation="create", width=12, height=8,
                    output_relative=blank_path)
    ink = drawing(41, operation="stroke", source_relative=blank_path,
                  output_relative=ink_path,
                  strokes=[{"radius": 0, "points": [[2, 2], [8, 5]]}])
    reopened = drawing(42, operation="capture", source_relative=ink_path)
    assert blank["content"][1]["type"] == "image", blank
    assert blank["content"][1]["data"] != ink["content"][1]["data"]
    assert reopened["content"][1] == ink["content"][1]
    rejected = drawing(43, operation="stroke", source_relative=ink_path,
                       output_relative=drawing_prefix + "/bad.json",
                       strokes=[{"radius": 0, "points": [[12, 2]]}])
    assert rejected["isError"] and not (project_root / drawing_prefix / "bad.json").exists()
    (project_root / ink_path).unlink()
    (project_root / blank_path).unlink()
    (project_root / drawing_prefix).rmdir()

    assets = tool(4, "list_assets", {"project_id": project_id})
    assert assets["ok"] and assets["result"], assets
    context = tool(5, "resolve_asset_context", {"project_id": project_id,
                   "asset_id": assets["result"][0]["asset_id"]})
    assert context["ok"] and context["result"]["texture"]["width"] > 0, context
    inspection = tool(6, "reference_operation", {"project_id":project_id,"request":{"operation": "inspect",
        "source_relative": ".assetlab/staging/long_shotgun_reference_v2/geometry.json"}})
    assert inspection["ok"], inspection
    traversal = tool(7, "reference_operation", {"project_id":project_id,"request":{"operation": "inspect",
                                                "source_relative": "../escape.json"}})
    assert not traversal["ok"], traversal
    collision = tool(8, "reference_operation", {"project_id":project_id,"request":{"operation": "create",
        "output_relative": ".assetlab/staging/long_shotgun_reference_v2"}})
    assert not collision["ok"], collision
    prefix = ".assetlab/staging/mcp_reference_smoke_" + uuid.uuid4().hex
    created = tool(9, "reference_operation", {"project_id": project_id, "request": {
        "operation": "create", "output_relative": prefix + "/base"}})
    assert created["ok"] and created["result"]["revision"] == 1, created
    dimensions = [{"id": "barrel-depth", "label": "Barrel depth",
                   "assignment": {"node_id": "barrel", "parameter": "depth",
                                  "value": 5.5}, "unit_label": "au"},
                  {"id": "trigger-angle", "label": "Trigger angle",
                   "assignment": {"node_id": "trigger", "parameter": {"rotation": "y"},
                                  "value": -10.0}, "unit_label": "°"}]
    applied = tool(10, "reference_operation", {"project_id": project_id, "request": {
        "operation": "apply", "source_relative": prefix + "/base/geometry.json",
        "output_relative": prefix + "/applied",
        "command": {"command": "replace_driving_dimensions",
                     "payload": {"dimensions": dimensions}}}})
    assert applied["ok"] and applied["result"]["revision"] == 2, applied
    undone = tool(11, "reference_operation", {"project_id": project_id, "request": {
        "operation": "undo", "source_relative": prefix + "/applied/geometry.json",
        "output_relative": prefix + "/undone"}})
    assert undone["ok"] and undone["result"]["revision"] == 3, undone
    redone = tool(12, "reference_operation", {"project_id": project_id, "request": {
        "operation": "redo", "source_relative": prefix + "/undone/geometry.json",
        "output_relative": prefix + "/redone"}})
    assert redone["ok"] and redone["result"]["revision"] == 4, redone
    rendered = tool(13, "reference_operation", {"project_id": project_id, "request": {
        "operation": "render", "source_relative": prefix + "/redone/geometry.json",
        "output_relative": prefix + "/rendered"}})
    assert rendered["ok"] and set(rendered["result"]["views"]) == {
        "side", "top", "front", "three_quarter"}, rendered
    sheet = (project_root / prefix / "rendered/technical.svg").read_text(encoding="utf-8")
    assert "Barrel depth: 5.500 au" in sheet and "Trigger angle: -10.000 °" in sheet
    for view in ("side", "top"):
        side = json.loads((project_root / prefix / f"rendered/{view}.projection.json").read_bytes())
        barrel = next(c for c in side["components"] if c["component_id"] == "barrel")
        projected_x = [x for x, _ in barrel["vertices"]]
        assert abs(max(projected_x) - min(projected_x) - 5.5) < 1e-9
    for identifier, (source, expected) in enumerate(
            (("base", 4.92), ("applied", 5.5), ("undone", 4.92), ("redone", 5.5)),
            start=20):
        inspected = tool(identifier, "reference_operation", {
            "project_id": project_id, "request": {
                "operation": "inspect", "source_relative": prefix + f"/{source}/geometry.json"}})
        assert inspected["ok"], inspected
        actual = inspected["result"]["spec"]["driving_dimensions"][0]["assignment"]["value"]
        assert actual == expected, (source, actual, expected)
    print(json.dumps({"status": "PASS", "tools": sorted(names),
                      "existing_context": "PASS", "reference_inspection": "PASS",
                      "saved_blender_frame_inline": "PASS",
                      "frame_path_and_signature_rejected": True,
                      "create_apply_undo_redo_render": "PASS",
                      "traversal_rejected": True, "collision_rejected": True}))
finally:
    process.terminate()
    process.wait(timeout=10)
    if "project_root" in locals() and "frame_prefix" in locals():
        target = (project_root / frame_prefix).resolve()
        project = project_root.resolve()
        if target != project and project in target.parents and target.name.startswith("mcp_reference_frame_smoke_"):
            shutil.rmtree(target, ignore_errors=True)
    if "project_root" in locals() and "prefix" in locals():
        target = (project_root / prefix).resolve()
        project = project_root.resolve()
        if target != project and project in target.parents and target.name.startswith("mcp_reference_smoke_"):
            shutil.rmtree(target, ignore_errors=True)
