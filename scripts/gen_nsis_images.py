"""Generate the NSIS installer bitmaps from the app icon.

NSIS needs BMP, not PNG, and MUI2 fixes the sizes: 150x57 for the header strip
and 164x314 for the welcome/finish sidebar. Both are written as 24-bit BMP
because NSIS cannot read an alpha channel.

The header sits on MUI2's white header background, so it is drawn on white to
blend; the sidebar owns its whole column, so it is drawn dark to match the app.

Run: python3 scripts/gen_nsis_images.py
"""

import pathlib

from PIL import Image, ImageDraw, ImageFont

ROOT = pathlib.Path(__file__).resolve().parent.parent
SOURCE = ROOT / "src-tauri/icons/128x128@2x.png"
OUT_DIR = ROOT / "src-tauri/installer"

WORDMARK = "AI TERM STUDIO"
HEADER_SIZE = (150, 57)
SIDEBAR_SIZE = (164, 314)

# Sampled from the icon's own background so the sidebar reads as an extension of
# the logo rather than a panel behind it.
SIDEBAR_TOP = (18, 20, 54)
SIDEBAR_BOTTOM = (38, 32, 92)


def load_font(size, bold=True):
    """Helvetica ships with macOS; fall back to PIL's bitmap font elsewhere."""
    for path in ("/System/Library/Fonts/HelveticaNeue.ttc", "/System/Library/Fonts/Helvetica.ttc"):
        try:
            return ImageFont.truetype(path, size, index=1 if bold else 0)
        except OSError:
            continue
    return ImageFont.load_default()


def logo(size):
    img = Image.open(SOURCE).convert("RGBA")
    return img.resize((size, size), Image.LANCZOS)


def flatten(rgba, background):
    """NSIS reads no alpha, so composite onto a solid colour first."""
    canvas = Image.new("RGB", rgba.size, background)
    canvas.paste(rgba, (0, 0), rgba)
    return canvas


def build_header():
    w, h = HEADER_SIZE
    img = Image.new("RGB", HEADER_SIZE, (255, 255, 255))
    mark = logo(40)
    img.paste(flatten(mark, (255, 255, 255)), (8, (h - 40) // 2), mark)

    draw = ImageDraw.Draw(img)
    font = load_font(10)
    text_y = (h - 10) // 2 - 2
    draw.text((56, text_y), WORDMARK, font=font, fill=(32, 32, 48))
    return img


def build_sidebar():
    w, h = SIDEBAR_SIZE
    img = Image.new("RGB", SIDEBAR_SIZE, SIDEBAR_TOP)
    draw = ImageDraw.Draw(img)
    for y in range(h):
        t = y / (h - 1)
        draw.line(
            [(0, y), (w, y)],
            fill=tuple(round(a + (b - a) * t) for a, b in zip(SIDEBAR_TOP, SIDEBAR_BOTTOM)),
        )

    # Placed so the logo-plus-wordmark group is optically centred: at y=62 the
    # group's centre sat 26px above the canvas centre and the lower third read
    # as dead space.
    mark = logo(104)
    img.paste(flatten(mark, SIDEBAR_TOP), ((w - 104) // 2, 88), mark)

    font = load_font(13)
    text_w = draw.textlength(WORDMARK, font=font)
    draw.text(((w - text_w) / 2, 212), WORDMARK, font=font, fill=(236, 238, 255))
    return img


def main():
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    for name, img in (("header", build_header()), ("sidebar", build_sidebar())):
        bmp = OUT_DIR / f"{name}.bmp"
        img.save(bmp, "BMP")
        # A PNG twin purely so the result can be eyeballed without a BMP viewer.
        img.save(OUT_DIR / f"{name}.preview.png", "PNG")
        print(f"{bmp.relative_to(ROOT)}  {img.size[0]}x{img.size[1]}  {bmp.stat().st_size} bytes")


if __name__ == "__main__":
    main()
