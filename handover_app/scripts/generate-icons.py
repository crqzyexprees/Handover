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

    icon_pngs = {
        16: png_bytes(image, 16, resample),
        32: png_bytes(image, 32, resample),
        64: png_bytes(image, 64, resample),
        128: png_bytes(image, 128, resample),
        256: png_bytes(image, 256, resample),
        512: png_bytes(image, 512, resample),
        1024: png_bytes(image, 1024, resample),
    }

    (ROOT / "32x32.png").write_bytes(icon_pngs[32])
    (ROOT / "128x128.png").write_bytes(icon_pngs[128])
    (ROOT / "128x128@2x.png").write_bytes(icon_pngs[256])
    (ROOT / "icon.png").write_bytes(icon_pngs[512])

    write_ico(
        ROOT / "icon.ico",
        [(16, icon_pngs[16]), (32, icon_pngs[32]), (256, icon_pngs[256])],
    )
    write_icns(
        ROOT / "icon.icns",
        [
            ("icp4", icon_pngs[16]),
            ("icp5", icon_pngs[32]),
            ("icp6", icon_pngs[64]),
            ("ic07", icon_pngs[128]),
            ("ic08", icon_pngs[256]),
            ("ic09", icon_pngs[512]),
            ("ic10", icon_pngs[1024]),
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
