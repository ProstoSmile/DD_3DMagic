"""Exercise the built CLI, Blender, durable reference publication, and tamper rejection."""
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib

repository = Path(__file__).resolve().parents[1]
scratch = repository / ".work/temp"
scratch.mkdir(parents=True, exist_ok=True)
root = Path(tempfile.mkdtemp(prefix="reference_smoke_", dir=scratch))
blender = Path(os.environ.get("DD_MAGIC_BLENDER", "D:/Programs/Blender/blender.exe"))
env = dict(os.environ, DD_MAGIC_PROJECT_ROOT=str(root), DD_MAGIC_BLENDER=str(blender),
           TEMP=str(scratch), TMP=str(scratch), PYTHONDONTWRITEBYTECODE="1")
cli = repository / "target/debug/dd-3dmagic.exe"


def run(name, request, succeeds=True):
    request_file = root / (name + ".json")
    request_file.write_text(json.dumps(request), encoding="utf-8")
    process = subprocess.run([str(cli), "reference", request_file.name], cwd=repository,
                             env=env, capture_output=True, text=True, timeout=195)
    if not succeeds:
        assert process.returncode != 0, process.stdout
        return process.stderr
    assert process.returncode == 0, process.stderr
    return json.loads(process.stdout)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


draft = ".assetlab/staging/create"
render = run("create", {"operation": "create", "output_relative": draft})
assert len(render["views"]) == 4
build = {
    "operation": "blender", "source_relative": draft + "/geometry.json",
    "output_relative": ".assetlab/staging/build", "category": "props",
    "subcategory": "weapons", "slug": "long_shotgun", "publish": True,
}
published = run("build", build)
assert published["published"]
assert published["comparison"]["status"] == "PASS"
assert len(published["comparison"]["renders"]) == 4
manifest = root / "assets/long_shotgun/asset.toml"
state = tomllib.loads(manifest.read_text(encoding="utf-8"))
geometry = state["technical"]["geometry_path"]
scene_relative = state["blend_path"]
scene = root / scene_relative
assert geometry.startswith("assets/long_shotgun/references/technical/")
assert digest(root / geometry) == state["technical"]["geometry_sha256"]
assert digest(scene) == next(item["sha256"] for item in state["source_artifacts"]
                              if item["artifact_id"] == "managed_proxy_scene")
assert [entry.name for entry in scene.parent.iterdir()] == ["long_shotgun.blend"]
before = digest(scene)

verify = {"operation": "verify_blender", "source_relative": geometry,
          "scene_relative": scene_relative, "output_relative": ".assetlab/staging/verify"}
assert run("verify", verify)["status"] == "PASS"
tampered = root / ".assetlab/staging/tampered.blend"
subprocess.run([str(blender), "--background", "--factory-startup", "--disable-autoexec",
                "--python-exit-code", "1", str(scene), "--python",
                str(repository / "tests/blender_reference_tamper.py"), "--", str(tampered)],
               cwd=root, env=env, capture_output=True, check=True, timeout=60)
assert "SAVE_VALIDATION_FAILED" in run("verify_tamper", {
    **verify, "scene_relative": ".assetlab/staging/tampered.blend",
    "output_relative": ".assetlab/staging/verify_tamper",
}, succeeds=False)
assert digest(scene) == before
print(json.dumps({"status": "PASS", "root": str(root), "components": published["comparison"]["components"],
                  "published_hash_verified": True, "reopened_render_count": 4,
                  "tamper_rejected": True, "published_scene_unchanged": True}))
