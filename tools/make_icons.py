"""Draws the Glint icon: an indigo-to-violet rounded square with a white four-point sparkle.
Writes rust/app/icons/{icon.ico, icon.png, 32x32.png}."""
import math, os
from PIL import Image, ImageDraw, ImageFilter

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.join(HERE, "..", "rust", "app", "icons")
os.makedirs(OUT, exist_ok=True)


def star(cx, cy, r_out, r_in, pts=4, rot=-math.pi / 2):
    out = []
    for i in range(pts * 2):
        r = r_out if i % 2 == 0 else r_in
        a = rot + i * math.pi / pts
        out.append((cx + r * math.cos(a), cy + r * math.sin(a)))
    return out


def draw(size):
    S = size * 4
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    grad = Image.new("RGBA", (S, S))
    gp = grad.load()
    a, b = (79, 70, 229), (168, 85, 247)
    for y in range(S):
        for x in range(S):
            t = (x * 0.35 + y * 0.65) / (S - 1)
            gp[x, y] = tuple(int(a[i] + (b[i] - a[i]) * t) for i in range(3)) + (255,)
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, S - 1, S - 1], radius=int(S * 0.24), fill=255)
    img.paste(grad, (0, 0), mask)
    # soft glow behind the sparkle
    glow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(glow).ellipse([S * 0.22, S * 0.22, S * 0.78, S * 0.78], fill=(255, 255, 255, 70))
    glow = glow.filter(ImageFilter.GaussianBlur(S * 0.08))
    img = Image.alpha_composite(img, Image.composite(glow, Image.new("RGBA", (S, S)), mask))
    d = ImageDraw.Draw(img)
    d.polygon(star(S * 0.5, S * 0.5, S * 0.34, S * 0.075), fill=(255, 255, 255, 255))
    d.polygon(star(S * 0.745, S * 0.27, S * 0.085, S * 0.022), fill=(255, 255, 255, 235))
    return img.resize((size, size), Image.LANCZOS)


big = draw(256)
big.save(os.path.join(OUT, "icon.png"))
draw(32).save(os.path.join(OUT, "32x32.png"))
big.save(os.path.join(OUT, "icon.ico"), sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
print("icons written to", os.path.abspath(OUT))
