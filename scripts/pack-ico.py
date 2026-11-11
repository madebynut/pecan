"""Pack lossless, separately rasterized PNG sizes into a Windows ICO container."""
from pathlib import Path
import struct
import sys

source, destination = map(Path, sys.argv[1:])
sizes = (16, 32, 48, 128, 256)
images = [(source / f"icon_{size}x{size}.png").read_bytes() for size in sizes]
offset = 6 + 16 * len(images)
entries = []
for size, data in zip(sizes, images):
    if data[:8] != b"\x89PNG\r\n\x1a\n" or struct.unpack(">II", data[16:24]) != (size, size):
        raise ValueError(f"Invalid PNG for ICO size {size}")
    entries.append(struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset))
    offset += len(data)
destination.write_bytes(struct.pack("<HHH", 0, 1, len(images)) + b"".join(entries + images))
