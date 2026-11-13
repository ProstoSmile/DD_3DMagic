"""Build one authored checkpoint through MCP and check returned Blender frames."""
import base64
import json
import os
from pathlib import Path
import subprocess
import sys

PROJECT = Path(__file__).resolve().parents[1]
REPO = PROJECT.parent


def tool(name, **arguments):
    environment = dict(os.environ, DD_MAGIC_PROJECT_ROOT=str(PROJECT))
    environment.pop("DD_MAGIC_WORKSPACE_ROOT", None)
    with subprocess.Popen(
        [REPO / "target/debug/dd-3dmagic-mcp.exe"], cwd=REPO, env=environment,
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, encoding="utf-8",
    ) as server:
        for message in (
            {"id": 1, "method": "initialize", "params": {
                "protocolVersion":"2024-11-05","capabilities":{},
                "clientInfo":{"name":"shotgun-checkpoints","version":"1"}}},
            {"method":"notifications/initialized"},
            {"id":2,"method":"tools/call","params":{"name":name,"arguments":arguments}},
        ):
            print(json.dumps(dict(message,jsonrpc="2.0")),file=server.stdin,flush=True)
        for line in server.stdout:
            response = json.loads(line)
            if response.get("id") == 2:
                server.stdin.close()
                if "error" in response or response["result"].get("isError"):
                    raise RuntimeError(response)
                return response["result"]["content"]
    raise RuntimeError("MCP stopped before returning a result")


def reference(request):
    result=json.loads(tool("reference_operation",request=request)[0]["text"])
    if not result["ok"]:
        raise RuntimeError(result)
    return result["result"]


def main(stage):
    artifact_name=f"shotgun_lowpoly_metric_v2_{stage}"
    root=f".assetlab/staging/{artifact_name}"
    request=json.loads((PROJECT/".assetlab/staging"/(artifact_name+"_request.json")).read_bytes())
    result=reference(request)
    assert set(result["views"])=={"side","top","front"}
    build=reference(dict(operation="blender",source_relative=result["directory"]+"/geometry.json",
                         output_relative=root,category="props",subcategory="weapons",
                         slug="long_shotgun",publish=False))
    assert build["comparison"]["status"]=="PASS"
    assert {view["view"] for view in build["comparison"]["renders"]}=={"side","top","front"}
    for view in ("side","top","front"):
        _,frame=tool("reference_frame",stage_relative=root,view=view)
        assert frame["type"]=="image" and frame["mimeType"]=="image/png"
        assert base64.b64decode(frame["data"],validate=True)==(PROJECT/root/f"blender_{view}.png").read_bytes()
    print(json.dumps({"stage":root,"scene":build["staged_scene"],
                      "gate":build["comparison"]["status"],"components":build["comparison"]["components"]}))


if __name__=="__main__":
    main(sys.argv[1])
