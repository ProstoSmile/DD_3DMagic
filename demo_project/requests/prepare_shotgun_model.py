"""Author the low-poly shotgun from stations in the approved ink reference."""
import hashlib
import json
import math
from pathlib import Path
import sys

PROJECT = Path(__file__).resolve().parents[1]
BASE = PROJECT / "assets/long_shotgun/references/design/orthographic_alignment_v1"
OVERALL_LENGTH_M = 1.20
BASELINE = json.loads((BASE / "baseline.json").read_bytes())
STATIONS = BASELINE["registration"]["stations_canvas_x"]
SCALE = OVERALL_LENGTH_M / (STATIONS["muzzle"] - STATIONS["butt"])


def vec(x, y, z):
    return {"x": (x - 185) * SCALE, "y": y * SCALE, "z": (58 - z) * SCALE}


def mesh(name, label, material, vertices, faces):
    volume = 0
    for face in faces:
        p = [tuple(vertices[index].values()) for index in face]
        for i in range(1, len(p) - 1):
            q, r = p[i], p[i + 1]
            volume += (p[0][0] * (q[1]*r[2] - q[2]*r[1]) +
                       p[0][1] * (q[2]*r[0] - q[0]*r[2]) +
                       p[0][2] * (q[0]*r[1] - q[1]*r[0]))
    if volume < 0:
        faces = [list(reversed(face)) for face in faces]
    if any(len(face) < 4 for face in faces):
        raise ValueError(f"{name}: authored mesh faces must be quads or n-gons")
    return {
        "id": name, "label": label, "material": material, "children": [],
        "transform": dict(translation=dict(x=0,y=0,z=0), rotation=dict(x=0,y=0,z=0),
                          scale=dict(x=1,y=1,z=1)),
        "primitive": {"kind":"polygon_mesh", "parameters":{
            "vertices":vertices, "faces":faces}},
    }


def connect(faces, a, b, n):
    for j in range(n):
        k = (j+1) % n
        faces.append([a+j,a+k,b+k,b+j])


def rings(name, label, mat, rings, tube=False):
    n = len(rings[0])
    faces = []
    for i in range(len(rings)-1):
        connect(faces,i*n,(i+1)*n,n)
    end = (len(rings)-1)*n
    if tube:
        connect(faces,end,0,n)
    else:
        faces.append(list(reversed(range(n))))
        faces.append([end+j for j in range(n)])
    return mesh(name,label,mat,[v for ring in rings for v in ring],faces)


def bevel(name,label,mat,stations):
    # X, top/bottom in the side drawing, half width in the aligned top drawing,
    # and one explicit corner cut. Each station describes a silhouette turn or join.
    sections = []
    for x,top,bottom,w,cut in stations:
        sections.append([vec(x,y,z) for y,z in [
            (-w+cut,top),(w-cut,top),(w,top+cut),(w,bottom-cut),
            (w-cut,bottom),(-w+cut,bottom),(-w,bottom-cut),(-w,top+cut),
        ]])
    return rings(name,label,mat,sections)


def round_section(name,label,stations,n,tube=False):
    sections = [[vec(x,r*math.cos(i*math.tau/n),51+r*math.sin(i*math.tau/n))
                 for i in range(n)] for x,r in stations]
    return rings(name,label,"dark_steel",sections,tube)


def ribbon(name,label,path,half_width):
    # Each bend is a projected silhouette station. The two rails of the
    # ribbon form one closed rectangular cross section, with no center vertices.
    sections = [[vec(a,-half_width,b),vec(c,-half_width,d),
                 vec(c,half_width,d),vec(a,half_width,b)] for a,b,c,d in path]
    return rings(name,label,"dark_steel",sections)


def main(stage):
    provenance = json.loads((BASE/"baseline.json").read_bytes())
    for name, entry in provenance["files"].items():
        assert hashlib.sha256((BASE/name).read_bytes()).hexdigest() == entry["sha256"], name
    components = [
        bevel("stock","Walnut stock","wood",[
            (22,46,96,12.4,2.8),(46,44,88,12,3.2),(76,47,77,10.5,3),
            (94,52,79,7.2,2.5),(109,52,88,6,2.4),(118,49,87,6.9,2.4),
            (126,45,79,8.3,2.8)]),
        bevel("receiver","Receiver","steel",[
            (120,48,82,7.3,2),(131,42,75,10.1,3),(145,40,71,11,3.5),
            (174,41,70,11,3.4),(185,44,67,9.8,2.7),(192,45,59,8.4,2.2)]),
        bevel("forearm","Walnut forearm","wood",[
            (187,57,70,7,2),(200,56,75,9.3,3),(226,56,76,10,3.5),
            (249,56,74,9.7,3.5),(266,56,71,8.4,2.5),(275,56.8,66,5.7,2)]),
        round_section("barrel","Barrel",[(190,6),(221,6),(378,4)],8),
        bevel("buttplate","Butt pad","rubber",[
            (13,53,83,8.2,3.3),(17,49,94,11.2,3.3),(22,46,96.5,12.8,3.1)]),
        ribbon("trigger-guard","Trigger guard",[
            (133,73,144,71),(135,84,143,82),(140,92,149,88),
            (151,96,155,90),(165,95,164,89),(176,91,172,86),
            (183,85,177,81),(183,74,176,74),(182,67,176,67)],3.4),
        ribbon("trigger","Trigger",[
            (156,69,162,69),(154.8,78,159.4,76),(156.5,83,159.6,81),
            (160,85.8,161.3,85.2)],1.4),
        round_section("muzzle","Muzzle collar",[(377.5,4),(380,5),(385,5),(387,3.4),
                                                (387,2.5),(378,2.5)],8,True),
        bevel("front-sight","Front sight","dark_steel",[
            (365,47,49,1.5,.25),(368,43,48,2,.55),(373,43,48,2,.55),(375,47,49,1.5,.25)]),
    ]
    if stage=="03_refined":
        components[0] = bevel("stock","Walnut stock","wood",[
            (22,46,96.5,13,2.8),(46,44,88.5,12.3,3.2),(76,47,76.5,10.7,3),
            (94,52,79,7.3,2.5),(109,52,88,6.2,2.4),(118,49,87,7.1,2.4),
            (126,45,79,8.3,2.8)])
        components[2] = bevel("forearm","Walnut forearm","wood",[
            (186,57,70,7,2),(200,56,75.5,9.5,3),(226,56,76.5,10,3),
            (249,56,74.5,10,3),(266,55.5,71,9,2.5),(276,56.1,66,6,1.6)])
        components[4] = bevel("buttplate","Butt pad","rubber",[
            (13,53,84,8.2,3),(17,48,94.5,11.8,3),(22,46,97,13,3.1)])
        components[8] = ribbon("front-sight","Front sight",
            [(365,47,368,43),(375,47,373,43)],2)
    if stage=="01_masses":
        components=components[:5]
    reference=PROJECT/"assets/long_shotgun/references/technical/r3_8f122803b18b_d4828858fa912/geometry.json"
    spec=json.loads(reference.read_bytes())
    spec.update(name="Long shotgun • authored metric polygon mesh",
                revision={"01_masses":7,"02_assembly":8,"03_refined":9}[stage],
                units="meters",
                technical_sheet_layout="orthographic_three",dimensions=[],
                driving_dimensions=[],constraints=[],
                parameter_provenance=[
                    {"path":"root.children","basis":"user_authored",
                     "explanation":"Hand-chosen stations aligned to approved side/top ink; front enlarged on source sheet."},
                    {"path":"units","basis":"user_authored",
                     "explanation":f"{OVERALL_LENGTH_M:.2f} m from butt to muzzle; converted from the shared side/top baseline stations; no UV or texel density asserted."}])
    spec["root"]["children"]=components
    stage_root=PROJECT/".assetlab/staging"
    artifact_name=f"shotgun_lowpoly_metric_v2_{stage}"
    request={"operation":"create","spec":spec,
             "output_relative":f".assetlab/staging/{artifact_name}_spec"}
    with (stage_root/f"{artifact_name}_request.json").open("x",encoding="utf-8") as f:
        json.dump(request,f,ensure_ascii=False,indent=2)


if __name__=="__main__":
    main(sys.argv[1])
