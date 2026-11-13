"""Overlay four registered reference and Blender views without resampling."""

import argparse
from pathlib import Path

from PIL import Image


VIEWS = ("front", "side", "top", "three_quarter")
BACKGROUND = (232, 232, 232, 255)


def load_view(path: Path) -> Image.Image:
    with Image.open(path) as source:
        if source.format != "PNG":
            raise ValueError(f"Expected a PNG view: {path}")
        image = source.convert("RGBA")
    background = Image.new("RGBA", image.size, BACKGROUND)
    return Image.alpha_composite(background, image).convert("RGB")


def compare_views(reference: Path, candidate: Path, output: Path) -> None:
    if output.exists():
        raise FileExistsError(f"Choose a new output directory: {output}")

    overlays = {}
    for view in VIEWS:
        expected = load_view(reference / f"{view}.png")
        actual = load_view(candidate / f"{view}.png")
        if expected.size != actual.size:
            raise ValueError(
                f"{view}: reference is {expected.size}, candidate is {actual.size}. "
                "Render at the reference resolution and framing."
            )
        overlays[view] = Image.blend(expected, actual, alpha=0.5)

    output.mkdir()
    for view, overlay in overlays.items():
        path = output / f"{view}_overlay.png"
        with path.open("xb") as stream:
            overlay.save(stream, format="PNG")
        print(path)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path, help="Directory of registered reference PNGs")
    parser.add_argument("candidate", type=Path, help="Directory of matching candidate PNGs")
    parser.add_argument("output", type=Path, help="New directory for the four overlays")
    args = parser.parse_args()
    try:
        compare_views(args.reference, args.candidate, args.output)
    except (OSError, ValueError) as error:
        parser.exit(1, f"{error}\n")


if __name__ == "__main__":
    main()
