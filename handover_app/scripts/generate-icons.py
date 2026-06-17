#!/usr/bin/env python3
"""Generate minimal Tauri icon PNGs (stdlib only). Brand purple #863bff."""
from __future__ import annotations

import struct
import zlib
from pathlib import Path

BRAND = (134, 59, 255)
ROOT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"


def write_png(path: Path, width: int, height: int, rgb: tuple[int, int, int]) -> None:
    def chunk(tag: bytes, data: bytes) -> bytes:
        crc = zlib.crc32(tag + data) & 0xFFFFFFFF
        return struct.pack(">I", len(data)) + tag + data + struct.pack(">I", crc)

    pixel = bytes((*rgb, 255))
    row = b"\x00" + pixel * width
    raw = row * height
    # Color type 6 = RGBA (required by Tauri).
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", ihdr)
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(png)


def write_minimal_ico(path: Path) -> None:
    # Single 32x32 32-bpp icon (simplified ICO container).
    png_path = path.parent / "_ico32.png"
    write_png(png_path, 32, 32, BRAND)
    png = png_path.read_bytes()
    png_path.unlink(missing_ok=True)
    header = struct.pack("<HHH", 0, 1, 1)
    entry = struct.pack("<BBBBHHII", 32, 32, 0, 0, 1, 32, len(png), 22)
    path.write_bytes(header + entry + png)


def main() -> None:
    write_png(ROOT / "icon.png", 512, 512, BRAND)
    write_png(ROOT / "32x32.png", 32, 32, BRAND)
    write_png(ROOT / "128x128.png", 128, 128, BRAND)
    write_png(ROOT / "128x128@2x.png", 256, 256, BRAND)
    write_minimal_ico(ROOT / "icon.ico")
    # macOS bundle uses icns; copy largest PNG as a stub for Linux dev builds.
    (ROOT / "icon.icns").write_bytes((ROOT / "icon.png").read_bytes())
    print(f"Wrote Tauri icons under {ROOT}")


if __name__ == "__main__":
    main()
