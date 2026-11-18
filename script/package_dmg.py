#!/usr/bin/env python3
"""Build Nasti's drag-to-Applications installer from a prepared macOS bundle."""

import argparse
from pathlib import Path
import subprocess
import tempfile

from dmgbuild import build_dmg


def render_background(artwork: Path, output: Path) -> None:
    # The README artwork is composited as-is. The opaque canvas lets Finder
    # render its native labels and targets in either system appearance.
    subprocess.run(
        [
            "magick", str(artwork),
            "-resize", "720x", "-gravity", "North",
            "-background", "#eeeae5", "-extent", "720x540",
            "(", "-size", "720x540", "gradient:#a8a8a8-#191919", ")",
            "-compose", "CopyOpacity", "-composite",
            "-background", "#eeeae5", "-alpha", "remove", "-alpha", "off",
            "-stroke", "#8c7764", "-strokewidth", "2.5", "-fill", "none",
            "-draw", "path 'M 315,395 L 405,395 M 391,383 L 405,395 L 391,407'",
            str(output),
        ],
        check=True,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("app", type=Path)
    parser.add_argument("dmg", type=Path)
    parser.add_argument("--volume-name", default="Nasti")
    args = parser.parse_args()
    app = args.app.resolve()
    if app.name != "Nasti.app" or not (app / "Contents/Info.plist").is_file():
        parser.error("app must be a prepared Nasti.app")
    if args.dmg.suffix != ".dmg" or args.dmg.exists():
        parser.error("provide a new .dmg output path")

    root = Path(__file__).resolve().parents[1]
    artwork = root / "docs/media/brand/nasti-character-logo-light.png"
    output = args.dmg.resolve()
    with tempfile.TemporaryDirectory(prefix="nasti-dmg-layout-", dir=output.parent) as scratch:
        background = Path(scratch) / "background.png"
        render_background(artwork, background)
        build_dmg(
            filename=str(output),
            volume_name=args.volume_name,
            settings={
                "format": "UDZO",
                "files": [str(app)],
                "symlinks": {"Applications": "/Applications"},
                "background": str(background),
                "window_rect": ((160, 130), (720, 540)),
                "icon_size": 112,
                "grid_spacing": 86,
                "text_size": 14,
                "icon_locations": {"Nasti.app": (202, 395), "Applications": (518, 395)},
                "hide_extensions": ["Nasti.app"],
            },
        )


if __name__ == "__main__":
    main()
