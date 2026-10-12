"""Companion's icon: WarpSign's swoosh with a down-arrow landing on it, on
teal-green.

    python3 tools/companion_icon.py

iOS's tinted and clear icons keep only an icon's shape (its brightness), so
Companion needs a different glyph from WarpSign's W, not just a different
colour. The arrow is drawn at the W's stroke weight; the swoosh and the
colours come from WarpSign's icon (its hue turned, and darkened, as green
reads much lighter than purple at the same value).
"""
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFilter

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "ios/WarpSign/Assets.xcassets/AppIcon.appiconset/AppIcon.png"
OUTPUT = ROOT / "ios/Companion/Assets.xcassets/AppIcon.appiconset/AppIcon.png"
SIZE = 1024
SCALE = 4  # drawn larger, then scaled down, for smooth edges
HUE_SHIFT = -85  # degrees
DARKEN = 0.42  # at full saturation
STROKE = 90  # the W's stroke width
SWOOSH_TOP = 750  # the swoosh is everything white below this


def recolour(rgb):
    """WarpSign's purple turned teal-green"""
    hsv = np.array(Image.fromarray(rgb).convert("HSV")).astype(np.float32)
    hsv[..., 0] = (hsv[..., 0] + HUE_SHIFT / 360 * 256) % 256
    hsv[..., 2] *= 1 - DARKEN * hsv[..., 1] / 255
    return np.array(Image.fromarray(np.clip(hsv, 0, 255).astype(np.uint8), "HSV").convert("RGB"))


def background(source):
    """A gradient between the source's corner colours, without its glyph"""
    corners = recolour(source[[24, 24, -25, -25], [24, -25, 24, -25]][None, :, :])[0].astype(np.float32)
    t = np.linspace(0, 1, SIZE, dtype=np.float32)
    x, y = np.meshgrid(t, t)
    top = corners[0] * (1 - x[..., None]) + corners[1] * x[..., None]
    bottom = corners[2] * (1 - x[..., None]) + corners[3] * x[..., None]
    return top * (1 - y[..., None]) + bottom * y[..., None]


def swoosh_mask(source):
    """The source's swoosh, by whiteness: its darkest channel, which stays
    well under 140 everywhere in the background"""
    alpha = np.clip((source.min(axis=2).astype(np.float32) - 140) / (255 - 140), 0, 1)
    alpha[:SWOOSH_TOP] = 0
    return alpha


def arrow_mask():
    """A down-arrow at the W's weight, over the swoosh"""
    big = SIZE * SCALE
    image = Image.new("L", (big, big), 0)
    draw = ImageDraw.Draw(image)
    width = STROKE * SCALE
    radius = width / 2

    def point(x, y):
        return (x * SCALE, y * SCALE)

    tip = (512, 676)
    shaft = [point(512, 300), point(*tip)]
    head = [point(332, 496), point(*tip), point(692, 496)]
    draw.line(shaft, fill=255, width=width)
    draw.line(head, fill=255, width=width, joint="curve")
    for x, y in shaft + head:
        draw.ellipse((x - radius, y - radius, x + radius, y + radius), fill=255)
    return np.array(image.resize((SIZE, SIZE), Image.LANCZOS)).astype(np.float32) / 255


def main():
    source = np.array(Image.open(SOURCE).convert("RGB"))
    glyph = np.maximum(arrow_mask(), swoosh_mask(source))

    # A soft shadow under the glyph, as WarpSign's has
    shadow = Image.fromarray((glyph * 255).astype(np.uint8)).filter(ImageFilter.GaussianBlur(18))
    shadow = np.roll(np.array(shadow).astype(np.float32) / 255, 10, axis=0)

    image = background(source) * (1 - 0.28 * shadow[..., None])
    image = image * (1 - glyph[..., None]) + 255 * glyph[..., None]
    Image.fromarray(np.clip(image, 0, 255).astype(np.uint8)).save(OUTPUT, optimize=True)


if __name__ == "__main__":
    main()
