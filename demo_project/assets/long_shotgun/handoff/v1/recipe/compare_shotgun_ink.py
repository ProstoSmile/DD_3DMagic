"""Compare saved Blender alpha with the independent approved side/top drawings."""
import json
from pathlib import Path
import sys

from PIL import Image, ImageDraw, ImageFont
import numpy as np

from prepare_shotgun_model import BASE, PROJECT, SCALE


def reference_mask(name):
    source = json.loads((BASE/f"{name}_strokes.json").read_bytes())
    strokes = [stroke["points"] for stroke in source["strokes"]]
    ink = Image.new("1",(source["width"],source["height"]))
    draw = ImageDraw.Draw(ink)
    draw.polygon(strokes[0],fill=1)
    if name == "side":
        for path in [
            strokes[1]+[[221,57],[192,58],[188,62]],
            strokes[2]+strokes[3][::-1],strokes[4],strokes[5],
        ]:
            draw.polygon(path,fill=1)
    return np.asarray(ink)


def registered_render(directory, view, record, size):
    alpha = Image.open(directory / f"blender_{view}.png").convert("RGBA").getchannel("A")
    width, height = alpha.size
    matrix = np.asarray(record["matrix_world"])
    position, right, up = matrix[:3, 3], matrix[:3, 0], matrix[:3, 1]
    factor = width / record["ortho_scale"]
    origin = np.asarray([-185*SCALE, 0, 58*SCALE])
    if view == "top":
        origin = np.asarray([-185*SCALE, 58*SCALE, 0])
    offset = origin - position
    registered = alpha.transform(size, Image.Transform.AFFINE,
        (factor*SCALE, 0, width/2+factor*np.dot(right,offset),
         0, factor*SCALE, height/2-factor*np.dot(up,offset)),
        resample=Image.Resampling.BILINEAR)
    return np.asarray(registered) >= 128


def main(stage):
    directory = PROJECT / ".assetlab/staging" / f"shotgun_lowpoly_{stage}"
    report = json.loads((directory / "comparison.json").read_bytes())
    records = {view["view"]:view for view in report["renders"]}
    results = {}
    panels = {}
    for view in ("side", "top"):
        target = reference_mask(view)
        actual = registered_render(directory,view,records[view],target.shape[::-1])
        union = target|actual
        intersection = target&actual
        results[view] = {"iou": float(intersection.sum()/union.sum()),
                         "ink_only_pixels": int((target&~actual).sum()),
                         "model_only_pixels": int((actual&~target).sum())}
        rgb=np.full((*target.shape,3),246,dtype=np.uint8)
        rgb[intersection] = [45,52,58]
        rgb[target&~actual] = [183,77,66]
        rgb[actual&~target] = [57,129,155]
        panels[view] = Image.fromarray(rgb)
        panels[view].save(directory/f"approved_{view}_overlay.png")
    results["front"] = {"status":"NEEDS_EVIDENCE",
                        "reason":"Approved front is an enlarged design drawing with unverified common scale."}
    with (directory/"approved_centerline_comparison.json").open("x") as f:
        json.dump(results,f,indent=2)
    page=Image.new("RGB",(1040,579),"#f6f6f6")
    draw=ImageDraw.Draw(page)
    font=lambda n: ImageFont.truetype("C:/Windows/Fonts/arial.ttf",n)
    draw.text((28,18),f"Shotgun / {stage} / independent silhouette check",font=font(26),fill="#202b34")
    draw.text((28,51),"Red: drawing only     Blue: model only     Dark: overlap",font=font(15),fill="#34434e")
    for i,(view,panel) in enumerate(panels.items()):
        y=79+i*224
        draw.text((28,y),f"{view.upper()}  {results[view]['iou']:.1%} overlap",font=font(16),fill="#202b34")
        page.paste(panel.crop((0,24,400,107)).resize((800,166),Image.Resampling.NEAREST),(28,y+23))
    draw.text((851,79),"FRONT · model",font=font(16),fill="#202b34")
    front=Image.open(directory/"blender_front.png").convert("RGBA")
    front=front.crop(front.getchannel("A").getbbox())
    front.thumbnail((150,205))
    page.paste(front,(885,110),front)
    draw.text((851,336),"Source front enlarged",font=font(14),fill="#34434e")
    draw.text((851,355),"Scale not registered",font=font(14),fill="#34434e")
    approved=Image.open(BASE/"front.png").convert("RGB").crop((31,14,85,106))
    page.paste(approved,(903,378))
    page.save(directory/"projection_centerline_review.png")
    print(json.dumps(results))


if __name__ == "__main__":
    main(sys.argv[1])
