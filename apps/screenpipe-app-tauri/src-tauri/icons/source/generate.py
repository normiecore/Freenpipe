#!/usr/bin/env python3
# Freenpipe — local-first screen & audio memory (fork of screenpipe @ 892199f, MIT)
# https://github.com/normiecore/Freenpipe
"""Regenerate every Freenpipe icon, installer image and in-app illustration.

The mark is a lowercase "f" with a dot (a recording light) on a rounded
square. All geometry lives in this file; it writes the SVG sources next to
itself, rasterises them with the Tauri CLI (`bunx tauri icon`, which renders
SVG), and composes the NSIS and DMG backgrounds with Pillow.

Run from apps/screenpipe-app-tauri:

    python3 src-tauri/icons/source/generate.py

Needs bun (with the app's node_modules installed) and Pillow. Only files that
already exist in the repository are overwritten, so the file layout the
bundle configs and the code refer to stays the same.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image, ImageColor, ImageDraw, ImageFont

SOURCE = Path(__file__).resolve().parent
ICONS = SOURCE.parent
TAURI = ICONS.parent
APP = TAURI.parent
ASSETS = TAURI / "assets"
PUBLIC = APP / "public"

GREEN = ("#34D399", "#047857")  # release tile, top-left to bottom-right
AMBER = ("#FBBF24", "#B45309")  # beta tile
INK = "#0B3B2E"  # text on light backgrounds

# Glyph strokes in a 1024 x 1024 box: the stem with its hooked top, the
# crossbar, and the dot to the lower right.
STEM = "M424 792 V408 A176 176 0 0 1 600 232 H648"
BAR = "M296 488 H592"
DOT = (704, 736, 72)


def glyph(color: str, stroke: int, badge: str) -> str:
    """SVG elements for the glyph. `badge` replaces the dot for tray states."""
    cx, cy, r = DOT
    parts = [
        f'<g fill="none" stroke="{color}" stroke-width="{stroke}" '
        'stroke-linecap="round" stroke-linejoin="round">',
        f'<path d="{STEM}"/>',
        f'<path d="{BAR}"/>',
        "</g>",
    ]
    if badge == "dot":
        parts.append(f'<circle cx="{cx}" cy="{cy}" r="{r}" fill="{color}"/>')
    elif badge == "ring":  # beta
        parts.append(
            f'<circle cx="{cx}" cy="{cy}" r="{r - 6}" fill="none" '
            f'stroke="{color}" stroke-width="40"/>'
        )
    elif badge == "cross":  # capture failed
        d = r - 8
        parts.append(
            f'<path d="M{cx - d} {cy - d} L{cx + d} {cy + d} M{cx + d} {cy - d} '
            f'L{cx - d} {cy + d}" stroke="{color}" stroke-width="44" '
            'stroke-linecap="round" fill="none"/>'
        )
    elif badge == "arrow":  # update available
        parts.append(
            f'<path d="M{cx} {cy + r} V{cy - r + 8} M{cx - 56} {cy - 16} '
            f'L{cx} {cy - r - 4} L{cx + 56} {cy - 16}" stroke="{color}" '
            'stroke-width="44" stroke-linecap="round" stroke-linejoin="round" '
            'fill="none"/>'
        )
    else:
        raise ValueError(badge)
    return "\n  ".join(parts)


def app_icon_svg(tile: tuple[str, str]) -> str:
    top, bottom = tile
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <defs>
    <linearGradient id="tile" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0" stop-color="{top}"/>
      <stop offset="1" stop-color="{bottom}"/>
    </linearGradient>
  </defs>
  <rect x="64" y="64" width="896" height="896" rx="208" fill="url(#tile)"/>
  {glyph("#FFFFFF", 112, "dot")}
</svg>
"""


def tray_svg(color: str, badge: str) -> str:
    # Crop to the glyph so it fills the small tray slot; strokes are heavier
    # than on the app icon so they survive 16 px rendering.
    return f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="144 152 720 720" width="720" height="720">
  {glyph(color, 128, badge)}
</svg>
"""


SVGS = {
    "freenpipe.svg": app_icon_svg(GREEN),
    "freenpipe-beta.svg": app_icon_svg(AMBER),
}
TRAY = {  # name: (color, badge)
    "tray-white": ("#FFFFFF", "dot"),
    "tray-black": ("#000000", "dot"),
    "tray-white-failed": ("#FFFFFF", "cross"),
    "tray-black-failed": ("#000000", "cross"),
    "tray-white-updates": ("#FFFFFF", "arrow"),
    "tray-black-updates": ("#000000", "arrow"),
    "tray-beta-white": ("#FFFFFF", "ring"),
    "tray-beta-black": ("#000000", "ring"),
}
for name, (color, badge) in TRAY.items():
    SVGS[f"{name}.svg"] = tray_svg(color, badge)

# Existing raster -> (svg source, pixel size). Tray icons keep upstream file
# names because the code loads them by path.
TRAY_FILES = {
    ASSETS / "screenpipe-logo-tray-white.png": ("tray-white.svg", 128),
    ASSETS / "screenpipe-logo-tray-black.png": ("tray-black.svg", 128),
    ASSETS / "screenpipe-logo-tray-white-failed.png": ("tray-white-failed.svg", 128),
    ASSETS / "screenpipe-logo-tray-black-failed.png": ("tray-black-failed.svg", 128),
    ASSETS / "screenpipe-logo-tray-updates-white.png": ("tray-white-updates.svg", 128),
    ASSETS / "screenpipe-logo-tray-updates-black.png": ("tray-black-updates.svg", 128),
    ASSETS / "screenpipe-logo-tray-beta-white.png": ("tray-beta-white.svg", 128),
    ASSETS / "screenpipe-logo-tray-beta-black.png": ("tray-beta-black.svg", 128),
    ICONS / "screenpipe-logo-tray-black.png": ("tray-black.svg", 256),
    ICONS / "screenpipe-logo-tray-failed.png": ("tray-black-failed.svg", 256),
    ICONS / "screenpipe-logo-tray-beta.png": ("tray-beta-black.svg", 256),
}
PUBLIC_FILES = {
    PUBLIC / "128x128.png": ("freenpipe.svg", 128),
    PUBLIC / "32x32.png": ("freenpipe.svg", 32),
    PUBLIC / "images" / "freenpipe.png": ("freenpipe.svg", 512),
}


def tauri_icon(svg: Path, out: Path, sizes: list[int] | None = None) -> None:
    cmd = ["bunx", "tauri", "icon", str(svg), "-o", str(out)]
    for size in sizes or []:
        cmd += ["-p", str(size)]
    done = subprocess.run(cmd, cwd=APP, capture_output=True, text=True)
    if done.returncode != 0:
        sys.stderr.write(done.stdout + done.stderr)
        raise SystemExit(f"tauri icon failed for {svg.name}")


def render(svg: Path, size: int, work: Path) -> Path:
    out = work / f"{svg.stem}-{size}"
    tauri_icon(svg, out, [size])
    return out / f"{size}x{size}.png"


def font(size: int, bold: bool = True) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    names = ["Inter-Bold.otf", "Inter-SemiBold.otf", "DejaVuSans-Bold.ttf"] if bold else [
        "Inter-Regular.otf",
        "DejaVuSans.ttf",
    ]
    for root in [Path("/usr/share/fonts"), Path("C:/Windows/Fonts")]:
        for name in names:
            hits = list(root.rglob(name)) if root.exists() else []
            if hits:
                return ImageFont.truetype(str(hits[0]), size)
    return ImageFont.load_default()


def vertical_gradient(size: tuple[int, int], top: str, bottom: str) -> Image.Image:
    w, h = size
    (r0, g0, b0), (r1, g1, b1) = ImageColor.getrgb(top), ImageColor.getrgb(bottom)
    img = Image.new("RGB", (w, h))
    draw = ImageDraw.Draw(img)
    for y in range(h):
        t = y / max(h - 1, 1)
        draw.line(
            [(0, y), (w, y)],
            fill=(round(r0 + (r1 - r0) * t), round(g0 + (g1 - g0) * t), round(b0 + (b1 - b0) * t)),
        )
    return img


def nsis_images(work: Path) -> None:
    mark = Image.open(render(SOURCE / "freenpipe.svg", 256, work)).convert("RGBA")

    # Header (shown at 2x: 332 x 126): white strip, mark and name at the right.
    header = Image.new("RGB", (332, 126), "#FFFFFF")
    m = mark.resize((84, 84), Image.LANCZOS)
    header.paste(m, (332 - 84 - 20, 21), m)
    draw = ImageDraw.Draw(header)
    f = font(34)
    text = "Freenpipe"
    tw = draw.textlength(text, font=f)
    draw.text((332 - 84 - 32 - tw, 42), text, font=f, fill=INK)
    header.save(ASSETS / "nsis-header.bmp")

    # Sidebar (164 x 314): green gradient, white glyph, name underneath.
    side = vertical_gradient((164, 314), GREEN[0], GREEN[1])
    glyph_png = Image.open(render(SOURCE / "tray-white.svg", 256, work)).convert("RGBA")
    g = glyph_png.resize((96, 96), Image.LANCZOS)
    side.paste(g, ((164 - 96) // 2, 92), g)
    draw = ImageDraw.Draw(side)
    f = font(24)
    tw = draw.textlength(text, font=f)
    draw.text(((164 - tw) / 2, 206), text, font=f, fill="#FFFFFF")
    side.save(ASSETS / "nsis-sidebar.bmp")


def dmg_background() -> None:
    w, h = 801, 455
    img = Image.new("RGBA", (w, h), "#F7FBF9")
    draw = ImageDraw.Draw(img)
    # Arrow between the app (left) and Applications (right) drop targets.
    y = 160
    draw.line([(300, y), (366, y)], fill=GREEN[1], width=10)
    draw.polygon([(366, y - 22), (398, y), (366, y + 22)], fill=GREEN[1])
    f = font(18, bold=False)
    text = "drag Freenpipe to Applications"
    tw = draw.textlength(text, font=f)
    draw.text((349 - tw / 2, y + 40), text, font=f, fill=INK)
    img.save(ASSETS / "dmg-background.png")


def oauth_walkthrough() -> None:
    """Two-step sketch of Google's unverified-app warning for BYO OAuth clients."""
    w, h = 1560, 712
    img = Image.new("RGB", (w, h), "#F4F4F5")
    draw = ImageDraw.Draw(img)
    title_font, body_font, small_font = font(30), font(20, bold=False), font(18)
    draw.text((w / 2, 28), "Google may show this warning for your own OAuth app", font=font(26),
              fill=INK, anchor="mt")

    def card(x: int, expanded: bool, step: str) -> None:
        y = 80
        draw.rounded_rectangle([x, y, x + 680, y + 600], radius=28, fill="#FFFFFF",
                               outline="#D4D4D8", width=3)
        # Warning sign.
        cx, ty = x + 90, y + 90
        draw.polygon([(cx, ty), (cx - 38, ty + 64), (cx + 38, ty + 64)], fill="#DC2626")
        draw.rectangle([cx - 4, ty + 20, cx + 4, ty + 44], fill="#FFFFFF")
        draw.ellipse([cx - 5, ty + 50, cx + 5, ty + 60], fill="#FFFFFF")
        draw.text((x + 52, y + 190), "Google hasn't verified this app", font=title_font, fill="#18181B")
        draw.text((x + 52, y + 245), "You created this OAuth client yourself, so this", font=body_font,
                  fill="#52525B")
        draw.text((x + 52, y + 273), "warning is expected.", font=body_font, fill="#52525B")
        link = "Hide Advanced" if expanded else "Advanced"
        draw.text((x + 52, y + 345), link, font=small_font, fill="#52525B")
        lw = draw.textlength(link, font=small_font)
        draw.line([(x + 52, y + 369), (x + 52 + lw, y + 369)], fill="#52525B", width=2)
        draw.rounded_rectangle([x + 470, y + 330, x + 640, y + 380], radius=6, fill="#2563EB")
        draw.text((x + 555, y + 355), "BACK TO SAFETY", font=font(16), fill="#FFFFFF", anchor="mm")
        if expanded:
            go = "Go to your app (unsafe)"
            draw.text((x + 52, y + 440), go, font=small_font, fill="#52525B")
            gw = draw.textlength(go, font=small_font)
            draw.line([(x + 52, y + 464), (x + 52 + gw, y + 464)], fill="#52525B", width=2)
            target = (x + 70, y + 480)
        else:
            target = (x + 70, y + 385)
        # Step marker pointing at the link to click.
        mx, my = target[0] + 30, target[1] + 70
        draw.line([target, (mx, my)], fill=GREEN[1], width=4)
        draw.ellipse([mx - 4, my, mx + 40, my + 44], fill=GREEN[1])
        draw.text((mx + 18, my + 22), step, font=font(22), fill="#FFFFFF", anchor="mm")
        draw.text((mx + 54, my + 22), "click here", font=font(22), fill=GREEN[1], anchor="lm")

    card(20, False, "1")
    card(860, True, "2")
    # Arrow between the two states.
    draw.line([(716, 380), (836, 380)], fill=GREEN[1], width=6)
    draw.polygon([(836, 366), (856, 380), (836, 394)], fill=GREEN[1])
    img.save(PUBLIC / "images" / "google-oauth-unverified-app-walkthrough.png", optimize=True)


def replace_existing(generated: Path, target: Path) -> None:
    for src in generated.rglob("*"):
        if not src.is_file():
            continue
        dst = target / src.relative_to(generated)
        if dst.exists():
            shutil.copyfile(src, dst)


def main() -> int:
    for name, text in SVGS.items():
        (SOURCE / name).write_text(text, encoding="utf-8", newline="\n")

    with tempfile.TemporaryDirectory() as tmp:
        work = Path(tmp)

        # Bundle icon sets (ico, icns, store tiles, mobile) plus the extra
        # square sizes the bundle configs list.
        for svg, target in [("freenpipe.svg", ICONS), ("freenpipe-beta.svg", ICONS / "beta")]:
            full = work / f"set-{target.name}"
            tauri_icon(SOURCE / svg, full)
            tauri_icon(SOURCE / svg, full, [256, 512, 1024])
            replace_existing(full, target)

        for target, (svg, size) in {**TRAY_FILES, **PUBLIC_FILES}.items():
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(render(SOURCE / svg, size, work), target)

        nsis_images(work)
    dmg_background()
    oauth_walkthrough()
    return 0


if __name__ == "__main__":
    sys.exit(main())
