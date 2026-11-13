"""Test-only mutation. The input scene is opened read-only and saved to a new copy."""
from pathlib import Path
import sys

import bpy


output = Path(sys.argv[sys.argv.index("--") + 1]).resolve()
if output.exists() or output.drive.lower() not in ("d:", "\\\\?\\d:"):
    raise ValueError("Expected a new D: test scene")
barrel = next(obj for obj in bpy.context.scene.objects if obj.get("dd_component_id") == "barrel")
barrel.location.z += 0.2
bpy.context.preferences.filepaths.save_version = 0
bpy.ops.wm.save_as_mainfile(filepath=str(output), check_existing=False)
