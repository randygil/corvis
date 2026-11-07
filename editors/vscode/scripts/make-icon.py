"""Draws media/icon.png (128x128): a steel "C" shaped like a bird head (Corviknight) with a red eye.

Run: python scripts/make-icon.py   (requires Pillow)
"""
import math
from pathlib import Path

from PIL import Image, ImageDraw

S = 1024  # drawing size, downsampled to 128 for anti-aliasing
OUT = Path(__file__).resolve().parent.parent / "media" / "icon.png"


def vertical_gradient(size, top, bottom):
    w, h = size
    img = Image.new("RGBA", size)
    px = img.load()
    for y in range(h):
        t = y / (h - 1)
        c = tuple(round(top[i] + (bottom[i] - top[i]) * t) for i in range(3)) + (255,)
        for x in range(w):
            px[x, y] = c
    return img


def main():
    # background: rounded square with a dark steel-blue gradient
    bg = vertical_gradient((S, S), (0x26, 0x36, 0x50), (0x0F, 0x17, 0x24))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle((0, 0, S - 1, S - 1), radius=210, fill=255)
    icon = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    icon.paste(bg, (0, 0), mask)

    cx, cy, R, r = 490, 540, 335, 205
    gap = 40  # half-opening of the "C", degrees

    def pt(radius, deg):
        a = math.radians(deg)
        return (cx + radius * math.cos(a), cy - radius * math.sin(a))

    # the "C": outer pie minus inner circle, opening to the right
    ring = Image.new("L", (S, S), 0)
    d = ImageDraw.Draw(ring)
    d.pieslice((cx - R, cy - R, cx + R, cy + R), start=gap, end=360 - gap, fill=255)
    d.ellipse((cx - r, cy - r, cx + r, cy + r), fill=0)
    # beak: continues the upper arm into a sharp hook
    d.polygon([pt(R, gap), (930, 470), pt(r, gap)], fill=255)

    steel = vertical_gradient((S, S), (0xE8, 0xEE, 0xF5), (0x8C, 0x9B, 0xAE))
    icon.paste(steel, (0, 0), ring)

    # red eye on the upper arm, near the beak
    ex, ey = pt((R + r) / 2, 62)
    er = 44
    eye = ImageDraw.Draw(icon)
    eye.ellipse((ex - er, ey - er, ex + er, ey + er), fill=(0xE5, 0x48, 0x4D, 255))
    hr = 13
    eye.ellipse((ex - er / 3 - hr, ey - er / 3 - hr, ex - er / 3 + hr, ey - er / 3 + hr), fill=(0xFF, 0xD6, 0xD6, 255))

    icon.resize((128, 128), Image.LANCZOS).save(OUT)
    print("wrote", OUT)


if __name__ == "__main__":
    main()
