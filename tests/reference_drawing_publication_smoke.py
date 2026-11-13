"""Publish the corrected ink study and its proxy in a disposable D: project."""

import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

repository = Path(__file__).resolve().parents[1]
demo = repository / "demo_project"
scratch = repository / ".work" / "temp"
scratch.mkdir(parents=True, exist_ok=True)
project = Path(tempfile.mkdtemp(prefix="drawing_publication_", dir=scratch))
geometry_input = ".assetlab/staging/shotgun_line_proxy_v3/geometry.json"
drawing_input = ".assetlab/staging/shotgun_line_study/07_level_detail.json"
for relative in (".assetlab/project.toml", geometry_input, drawing_input):
    destination = project / relative
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes((demo / relative).read_bytes())

environment = dict(os.environ, DD_MAGIC_PROJECT_ROOT=str(project),
                   TEMP=str(scratch), TMP=str(scratch))
environment.pop("DD_MAGIC_WORKSPACE_ROOT", None)
initialize = {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
    "protocolVersion": "2024-11-05", "capabilities": {},
    "clientInfo": {"name": "drawing-publication-smoke", "version": "1"},
}}
initialized = {"jsonrpc": "2.0", "method": "notifications/initialized"}


def tool(name, request):
    invocation = {"jsonrpc": "2.0", "id": 2, "method": "tools/call",
                  "params": {"name": name, "arguments": {"request": request}}}
    # Keep stdin open until the tool returns; closing it earlier cancels the call.
    with subprocess.Popen(
        [repository / "target/debug/dd-3dmagic-mcp.exe"], cwd=repository, env=environment,
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
    ) as server:
        for message in (initialize, initialized, invocation):
            print(json.dumps(message), file=server.stdin, flush=True)
        for line in server.stdout:
            response = json.loads(line)
            if response.get("id") == 2:
                server.stdin.close()
                assert "error" not in response and not response["result"].get("isError"), response
                return response["result"]["content"]
    raise AssertionError("MCP stopped before returning the tool result")


def reference(*, succeeds=True, **request):
    response = json.loads(tool("reference_operation", request)[0]["text"])
    if not succeeds:
        assert not response["ok"], response
        return response["error"]
    assert response["ok"], response
    return response["result"]


def sha(path):
    return hashlib.sha256((project / path).read_bytes()).hexdigest()


build = dict(
    operation="blender", source_relative=geometry_input, drawing_reference_relative=drawing_input,
    category="props", subcategory="weapons", slug="long_shotgun",
)
staged = reference(**build, output_relative=".assetlab/staging/review")
reviewed = dict(scene_relative=staged["staged_scene"], scene_sha256=sha(staged["staged_scene"]),
                geometry_sha256=sha(geometry_input), drawing_sha256=sha(drawing_input))
rejection = reference(**build, output_relative=".assetlab/staging/rejected", publish=True,
                      reviewed_proxy={**reviewed, "scene_sha256": "0" * 64}, succeeds=False)
assert rejection["code"] == "SAVE_VALIDATION_FAILED"
assert not (project / ".assetlab/staging/rejected").exists()
published = reference(**build, output_relative=".assetlab/staging/publish",
                      publish=True, reviewed_proxy=reviewed)
assert published["published"] and published["comparison"]["status"] == "PASS"
manifest = tomllib.loads((project / "assets/long_shotgun/asset.toml").read_text(encoding="utf-8"))
reference_id = manifest["active_references"]["design_reference"]
design = next(r for r in manifest["references"] if r["reference_id"] == reference_id)
assert design["path"] == published["design_drawing"]
assert design["sha256"] == sha(design["path"])
strokes = next(a for a in manifest["source_artifacts"] if a["artifact_id"] == "design_strokes")
assert strokes["path"] == published["design_strokes"]
assert strokes["sha256"] == sha(drawing_input) == sha(strokes["path"])
provenance = json.loads((project / design["path"]).with_name("design_drawing_provenance.json").read_bytes())
assert provenance["source_sha256"] == strokes["sha256"]
assert provenance["preview_sha256"] == design["sha256"]
assert provenance["geometry_sha256"] == sha(published["geometry_path"])
assert (project / design["path"]).read_bytes() == (
    demo / ".assetlab/staging/shotgun_line_study/long_shotgun_line_reference_v2.png"
).read_bytes()

# Show that the manifest and canvas remain usable after the scratch input is removed.
(project / drawing_input).unlink()
capture = tool("reference_drawing", {"operation": "capture", "source_relative": strokes["path"]})
assert base64.b64decode(capture[1]["data"]) == (project / design["path"]).read_bytes()
scene = manifest["blend_path"]
assert scene == "blender/models/props/weapons/long_shotgun/long_shotgun.blend"
scene_before = sha(scene)
assert scene_before == reviewed["scene_sha256"] == sha(reviewed["scene_relative"])
saved = reference(operation="verify_blender", source_relative=published["geometry_path"],
                  scene_relative=scene, output_relative=".assetlab/staging/verify")
assert saved["status"] == "PASS" and sha(scene) == scene_before
print(json.dumps({"status": "PASS", "project": str(project), "scene": scene,
                  "design": design["path"], "strokes": strokes["path"]}))
