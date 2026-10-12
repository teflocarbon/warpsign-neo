"""Companion's icon: WarpSign's, turned teal-green.

    python3 tools/companion_icon.py

Rotates the hue of ios/WarpSign/Assets.xcassets/AppIcon.appiconset/AppIcon.png
and darkens its colours (not the white glyph, which has no saturation), as
green reads much lighter than purple at the same value.
"""
from pathlib import Path

import numpy as np
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "ios/WarpSign/Assets.xcassets/AppIcon.appiconset/AppIcon.png"
OUTPUT = ROOT / "ios/Companion/Assets.xcassets/AppIcon.appiconset/AppIcon.png"
HUE_SHIFT = -85  # degrees
DARKEN = 0.42  # at full saturation


def main():
    hsv = np.array(Image.open(SOURCE).convert("RGB").convert("HSV")).astype(np.float32)
    hsv[..., 0] = (hsv[..., 0] + HUE_SHIFT / 360 * 256) % 256
    hsv[..., 2] *= 1 - DARKEN * hsv[..., 1] / 255
    image = Image.fromarray(np.clip(hsv, 0, 255).astype(np.uint8), "HSV").convert("RGB")
    image.save(OUTPUT, optimize=True)


if __name__ == "__main__":
    main()
