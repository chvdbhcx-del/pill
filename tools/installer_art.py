"""Draws the Pill installer artwork (needs Pillow): src-tauri/installer/sidebar.bmp (164x314) and header.bmp (150x57).

    python tools/installer_art.py

Drawn from scratch with code, the app icon is src-tauri/icons/icon.png. The output files are committed; run this only to change the look.
"""
import os
from PIL import Image, ImageDraw, ImageFilter, ImageFont

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "src-tauri", "installer")
FONTS = os.path.join(ROOT, "src", "fonts")
os.makedirs(OUT, exist_ok=True)


def lerp(a, b, t):
    return tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(len(a)))


def gradient(w, h, stops):
    g = Image.new("RGB", (w, h))
    px = g.load()
    for y in range(h):
        for x in range(w):
            t = x / max(w - 1, 1) * 0.3 + y / max(h - 1, 1) * 0.7
            for i in range(len(stops) - 1):
                (t0, c0), (t1, c1) = stops[i], stops[i + 1]
                if t <= t1 or i == len(stops) - 2:
                    px[x, y] = lerp(c0, c1, min(max((t - t0) / (t1 - t0), 0), 1))
                    break
    return g


def blob(img, cx, cy, r, color, alpha):
    layer = Image.new("RGBA", img.size, (0, 0, 0, 0))
    ImageDraw.Draw(layer).ellipse((cx - r, cy - r, cx + r, cy + r), fill=color + (alpha,))
    img.alpha_composite(layer.filter(ImageFilter.GaussianBlur(r * 0.55)))


def font(name, px):
    for f in (os.path.join(FONTS, name), r"C:\Windows\Fonts\segoeui.ttf"):
        try:
            return ImageFont.truetype(f, px)
        except OSError:
            pass
    return ImageFont.load_default()


def icon(px):
    return Image.open(os.path.join(ROOT, "src-tauri", "icons", "icon.png")).convert("RGBA").resize((px, px), Image.LANCZOS)


def sidebar():
    s = 3
    w, h = 164 * s, 314 * s
    img = gradient(w, h, [(0, (12, 12, 28)), (0.55, (52, 44, 150)), (1, (108, 96, 245))]).convert("RGBA")
    blob(img, w * 0.9, h * 0.15, w * 0.6, (150, 140, 255), 120)
    blob(img, w * 0.1, h * 0.9, w * 0.7, (255, 120, 200), 90)
    d = ImageDraw.Draw(img)
    # the island: a black capsule with a glow underneath, a ring and a dot
    glow = Image.new("RGBA", img.size, (0, 0, 0, 0))
    ImageDraw.Draw(glow).rounded_rectangle((22 * s, 112 * s, 142 * s, 150 * s), 19 * s, fill=(170, 150, 255, 150))
    img.alpha_composite(glow.filter(ImageFilter.GaussianBlur(7 * s)))
    d.rounded_rectangle((18 * s, 100 * s, 146 * s, 138 * s), 19 * s, fill=(6, 6, 12, 255))
    d.arc((28 * s, 110 * s, 46 * s, 128 * s), -90, 200, fill=(190, 180, 255, 255), width=2 * s)
    d.text((54 * s, 110 * s), "Pill", font=font("Nunito-SemiBold.ttf", 15 * s), fill=(255, 255, 255, 255))
    d.ellipse((124 * s, 114 * s, 134 * s, 124 * s), fill=(255, 150, 90, 255))
    img.alpha_composite(icon(64 * s), (14 * s, 22 * s))
    d.text((14 * s, 254 * s), "Pill", font=font("Nunito-Bold.ttf", 26 * s), fill=(255, 255, 255, 255))
    d.text((15 * s, 254 * s + 31 * s), "dynamic island", font=font("Nunito-Light.ttf", 12 * s), fill=(235, 232, 255, 235))
    return img.convert("RGB").resize((164, 314), Image.LANCZOS)


def header():
    s = 4
    img = Image.new("RGBA", (150 * s, 57 * s), (255, 255, 255, 255))
    img.alpha_composite(icon(40 * s), (100 * s, 8 * s))
    return img.convert("RGB").resize((150, 57), Image.LANCZOS)


if __name__ == "__main__":
    sidebar().save(os.path.join(OUT, "sidebar.bmp"))
    header().save(os.path.join(OUT, "header.bmp"))
    print("wrote", sorted(os.listdir(OUT)))
