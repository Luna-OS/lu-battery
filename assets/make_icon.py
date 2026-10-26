"""Generates the lu-battery app icon (Moon theme) without any third-party code.

    python assets/make_icon.py

Writes assets/icon.png (256 px) and assets/icon.ico (16, 32, 48, 256; PNG entries).
Motif: a lavender crescent moon holding a small mint battery, on the Moon
background. Every size is rendered with signed distance functions at 4x and
box-filtered down, so small sizes stay crisp.
"""

import math
import os
import struct
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
SS = 4  # supersampling factor


def hex_rgb(h):
    return tuple(int(h[i : i + 2], 16) / 255 for i in (1, 3, 5))


BG_TOP = hex_rgb("#1d1742")  # surfaceFocus
BG_BOTTOM = hex_rgb("#0b0920")  # bg
BORDER = hex_rgb("#5b4f9e")  # outline
MOON_LIGHT = hex_rgb("#f4f1ff")  # moon100
MOON_DARK = hex_rgb("#b9aefb")  # lavender400
MINT = hex_rgb("#7fe3c6")  # success


def lerp(a, b, t):
    return tuple(x + (y - x) * t for x, y in zip(a, b))


def sd_circle(x, y, cx, cy, r):
    return math.hypot(x - cx, y - cy) - r


def sd_round_box(x, y, cx, cy, hw, hh, r):
    qx = abs(x - cx) - hw + r
    qy = abs(y - cy) - hh + r
    return math.hypot(max(qx, 0.0), max(qy, 0.0)) + min(max(qx, qy), 0.0) - r


def coverage(d, px):
    return min(1.0, max(0.0, 0.5 - d / px))


def render(size):
    n = size * SS
    px = 1.0 / n
    # Small sizes get thicker strokes so they survive the downscale.
    small = size <= 32
    stroke = 0.050 if small else 0.034
    pixels = []  # premultiplied rgba floats
    for j in range(n):
        y = (j + 0.5) / n
        for i in range(n):
            x = (i + 0.5) / n
            r = g = b = a = 0.0

            def over(color, alpha):
                nonlocal r, g, b, a
                if alpha <= 0.0:
                    return
                k = 1.0 - alpha
                r = color[0] * alpha + r * k
                g = color[1] * alpha + g * k
                b = color[2] * alpha + b * k
                a = alpha + a * k

            # Background squircle with a vertical gradient and a thin rim.
            d_bg = sd_round_box(x, y, 0.5, 0.5, 0.47, 0.47, 0.22)
            c_bg = coverage(d_bg, px)
            if c_bg <= 0.0:
                pixels.append((0.0, 0.0, 0.0, 0.0))
                continue
            over(lerp(BG_TOP, BG_BOTTOM, y), c_bg)
            rim = 0.016 if small else 0.010
            over(BORDER, coverage(abs(d_bg + rim) - rim, px) * c_bg * 0.9)

            # Crescent: big disc minus an offset disc.
            d_moon = max(
                sd_circle(x, y, 0.46, 0.52, 0.32),
                -sd_circle(x, y, 0.64, 0.40, 0.27),
            )
            t = min(1.0, max(0.0, (x + y - 0.5) / 0.9))
            over(lerp(MOON_LIGHT, MOON_DARK, t), coverage(d_moon, px))

            # Battery in the crescent's hollow: outline, nub, fill level.
            bx, by, bw, bh = 0.665, 0.56, 0.095, 0.155
            d_body = sd_round_box(x, y, bx, by, bw, bh, 0.045)
            over(MINT, coverage(abs(d_body) - stroke / 2, px))
            d_nub = sd_round_box(x, y, bx, by - bh - 0.035, 0.045, 0.028, 0.015)
            over(MINT, coverage(d_nub, px))
            inset = stroke / 2 + (0.030 if small else 0.028)
            level_top = by - bh + inset + (2 * (bh - inset)) * (0.0 if size <= 16 else 0.35)
            fill_cy = (level_top + by + bh - inset) / 2
            fill_hh = (by + bh - inset - level_top) / 2
            d_fill = sd_round_box(x, y, bx, fill_cy, bw - inset, fill_hh, 0.02)
            over(MINT, coverage(d_fill, px))

            pixels.append((r, g, b, a))

    # Box filter SS x SS down to `size`, then unpremultiply.
    out = bytearray()
    for j in range(size):
        out.append(0)  # PNG filter type: none
        for i in range(size):
            acc = [0.0, 0.0, 0.0, 0.0]
            for dj in range(SS):
                row = (j * SS + dj) * n
                for di in range(SS):
                    p = pixels[row + i * SS + di]
                    for k in range(4):
                        acc[k] += p[k]
            acc = [v / (SS * SS) for v in acc]
            alpha = acc[3]
            if alpha > 0:
                rgb = [min(1.0, c / alpha) for c in acc[:3]]
            else:
                rgb = [0.0, 0.0, 0.0]
            out += bytes(round(c * 255) for c in rgb) + bytes([round(alpha * 255)])
    return encode_png(size, size, bytes(out))


def encode_png(w, h, raw):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def encode_ico(images):
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    entries = b""
    data = b""
    for size, png in images:
        dim = 0 if size >= 256 else size
        entries += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(png), offset + len(data))
        data += png
    return header + entries + data


def main():
    images = [(s, render(s)) for s in (16, 32, 48, 256)]
    with open(os.path.join(HERE, "icon.ico"), "wb") as f:
        f.write(encode_ico(images))
    with open(os.path.join(HERE, "icon.png"), "wb") as f:
        f.write(images[-1][1])
    for s, png in images:
        with open(os.path.join(HERE, f"icon-{s}.png"), "wb") as f:
            f.write(png)
    print("wrote icon.ico, icon.png, icon-{16,32,48,256}.png")


if __name__ == "__main__":
    main()
