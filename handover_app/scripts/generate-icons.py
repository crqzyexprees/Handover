#!/usr/bin/env python3
"""Generate Tauri icons from src-tauri/icons/icon-source.png."""
from __future__ import annotations

import io
import struct
import zlib
from pathlib import Path

BRAND = (134, 59, 255)
ROOT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"
SOURCE = ROOT / "icon-source.png"


def write_solid_png(path: Path, width: int, height: int) -> None:
    def chunk(tag: bytes, data: bytes) -> bytes:
        crc = zlib.crc32(tag + data) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", crc)

    pixel = bytes((*BRAND, 255))
    row = b"\x00" + pixel * width
    raw = row * height
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png)


def png_bytes(image, size: int, resample) -> bytes:
    resized = image.resize((size, size), resample)
    out = io.BytesIO()
    resized.save(out, format="PNG")
    return out.getvalue()


# The handwritten "H" in icon-source.png (x0, y0, x1, y1). At 64px and below
# the whole word is unreadable, so small icons show just this letter.
H_GLYPH_BOX = (200, 488, 293, 632)
SMALL_ICON_MAX = 64


def monogram(image, size: int, resample):
    """The source's "H" alone, centred on its paper colour, for tiny sizes."""
    from PIL import Image, ImageFilter, ImageStat

    paper = tuple(int(c) for c in ImageStat.Stat(image.crop((80, 80, 944, 400)).convert("RGB")).median)
    glyph = image.crop(H_GLYPH_BOX).convert("L")
    # Ink coverage: dark strokes -> opaque, paper -> transparent.
    ink = glyph.point(lambda v: max(0, min(255, (130 - v) * 5)))
    # Fill ~78% of the icon's height.
    target_h = round(size * 0.78)
    target_w = max(1, round(glyph.width * target_h / glyph.height))
    # Thicken strokes so they survive downscaling to a pixel or two.
    ink = ink.filter(ImageFilter.MaxFilter(9 if size <= 32 else 5))
    ink = ink.resize((target_w, target_h), resample)
    canvas = Image.new("RGBA", (size, size), (*paper, 255))
    letter = Image.new("RGBA", (target_w, target_h), (17, 17, 17, 255))
    canvas.paste(letter, ((size - target_w) // 2, (size - target_h) // 2), ink)
    return canvas


def icon_png(image, size: int, resample) -> bytes:
    if size <= SMALL_ICON_MAX:
        out = io.BytesIO()
        monogram(image, size, resample).save(out, format="PNG")
        return out.getvalue()
    return png_bytes(image, size, resample)


def write_ico(path: Path, pngs: list[tuple[int, bytes]]) -> None:
    header = struct.pack("<HHH", 0, 1, len(pngs))
    offset = 6 + 16 * len(pngs)
    entries = []
    payload = []
    for size, data in pngs:
        dim = 0 if size >= 256 else size
        entries.append(
            struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(data), offset)
        )
        payload.append(data)
        offset += len(data)
    path.write_bytes(header + b"".join(entries) + b"".join(payload))


def write_icns(path: Path, pngs: list[tuple[str, bytes]]) -> None:
    chunks = []
    total = 8
    for icon_type, data in pngs:
        encoded_type = icon_type.encode("ascii")
        chunk = encoded_type + struct.pack(">I", len(data) + 8) + data
        chunks.append(chunk)
        total += len(chunk)
    path.write_bytes(b"icns" + struct.pack(">I", total) + b"".join(chunks))


def generate_from_source() -> bool:
    if not SOURCE.exists():
        return False
    try:
        from PIL import Image
    except ImportError:
        return False

    resample = getattr(Image, "Resampling", Image).LANCZOS
    with Image.open(SOURCE) as original:
        image = original.convert("RGBA")

    icon_pngs = {size: icon_png(image, size, resample) for size in (16, 32, 48, 64, 128, 256, 512, 1024)}

    (ROOT / "32x32.png").write_bytes(icon_pngs[32])
    (ROOT / "128x128.png").write_bytes(icon_pngs[128])
    (ROOT / "128x128@2x.png").write_bytes(icon_pngs[256])
    (ROOT / "icon.png").write_bytes(icon_pngs[512])

    public_dir = ROOT.parent.parent / "public"
    public_dir.mkdir(parents=True, exist_ok=True)
    (public_dir / "favicon.png").write_bytes(icon_pngs[128])

    write_ico(
        ROOT / "icon.ico",
        [(16, icon_pngs[16]), (32, icon_pngs[32]), (48, icon_pngs[48]), (256, icon_pngs[256])],
    )
    write_icns(
        ROOT / "icon.icns",
        # The PNG-based types iconutil writes: 1x and @2x for each size.
        [
            ("icp4", icon_pngs[16]),
            ("ic11", icon_pngs[32]),  # 16@2x
            ("icp5", icon_pngs[32]),
            ("ic12", icon_pngs[64]),  # 32@2x
            ("ic07", icon_pngs[128]),
            ("ic13", icon_pngs[256]),  # 128@2x
            ("ic08", icon_pngs[256]),
            ("ic14", icon_pngs[512]),  # 256@2x
            ("ic09", icon_pngs[512]),
            ("ic10", icon_pngs[1024]),  # 512@2x
        ],
    )
    return True


def generate_fallback() -> None:
    write_solid_png(ROOT / "icon.png", 512, 512)
    write_solid_png(ROOT / "32x32.png", 32, 32)
    write_solid_png(ROOT / "128x128.png", 128, 128)
    write_solid_png(ROOT / "128x128@2x.png", 256, 256)
    write_ico(ROOT / "icon.ico", [(32, (ROOT / "32x32.png").read_bytes())])
    write_icns(ROOT / "icon.icns", [("ic09", (ROOT / "icon.png").read_bytes())])


def main() -> None:
    ROOT.mkdir(parents=True, exist_ok=True)
    if not generate_from_source():
        generate_fallback()
    print(f"Wrote Tauri icons under {ROOT}")


if __name__ == "__main__":
    main()
