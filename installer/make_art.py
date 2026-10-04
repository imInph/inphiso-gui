"""Generates installer artwork in the app's palette. Run: python3 installer/make_art.py"""
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).parent
BG, INK, ACCENT, MUTED, BORDER = "#e3ebf0", "#0b2230", "#17898a", "#5b6b77", "#c3ced8"

def font(size, bold=False):
    for path in ["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Helvetica.ttc",
                 "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf" if bold else "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"]:
        try:
            return ImageFont.truetype(path, size)
        except OSError:
            continue
    return ImageFont.load_default()

def logo(draw, x, y, s):
    """The download-arrow mark from the app header, s pixels square."""
    draw.rounded_rectangle([x, y, x + s, y + s], radius=s * 0.27, fill=ACCENT)
    k = s / 24
    w = max(2, round(2.6 * k))
    p = lambda px, py: (x + s / 2 + (px - 12) * k * 0.62, y + s / 2 + (py - 12) * k * 0.62)
    draw.line([p(12, 3), p(12, 15)], fill=BG, width=w)
    draw.line([p(6, 10), p(12, 16), p(18, 10)], fill=BG, width=w, joint="curve")
    draw.line([p(5, 21), p(19, 21)], fill=BG, width=w)

def splash():
    """Shown by the Windows installer while it starts (advsplash, BMP)."""
    w, h = 480, 280
    img = Image.new("RGB", (w, h), BG)
    d = ImageDraw.Draw(img)
    d.rectangle([0, 0, w - 1, h - 1], outline=BORDER)
    logo(d, w // 2 - 36, 64, 72)
    f = font(34, bold=True)
    text = "inphiso"
    tw = d.textlength(text, font=f)
    d.text(((w - tw) / 2, 156), text, fill=INK, font=f)
    sub = "Installing…"
    fs = font(15)
    d.text(((w - d.textlength(sub, font=fs)) / 2, 206), sub, fill=MUTED, font=fs)
    img.save(HERE / "windows" / "splash.bmp")

def header():
    """Top-right of the installer's progress window (150x57 BMP)."""
    img = Image.new("RGB", (150, 57), "#ffffff")
    d = ImageDraw.Draw(img)
    logo(d, 104, 12, 32)
    img.save(HERE / "windows" / "header.bmp")

def dmg_background(scale):
    """DMG window: app on the left, arrow to Applications on the right."""
    w, h = 660 * scale, 400 * scale
    img = Image.new("RGB", (w, h), BG)
    d = ImageDraw.Draw(img)
    y = 190 * scale
    x0, x1 = 250 * scale, 410 * scale
    d.line([(x0, y), (x1, y)], fill=ACCENT, width=4 * scale)
    d.polygon([(x1 + 16 * scale, y), (x1 - 4 * scale, y - 13 * scale), (x1 - 4 * scale, y + 13 * scale)], fill=ACCENT)
    f = font(15 * scale)
    msg = "Drag inphiso to Applications"
    d.text(((w - d.textlength(msg, font=f)) / 2, 330 * scale), msg, fill=MUTED, font=f)
    name = "background.png" if scale == 1 else "background@2x.png"
    img.save(HERE / "macos" / name)

splash()
header()
dmg_background(1)
dmg_background(2)
print("artwork written")
