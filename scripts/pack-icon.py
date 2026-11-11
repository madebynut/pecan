"""Pack legacy RGB/alpha thumbnails and PNG Retina representations in ICNS.

macOS iconutil interprets icp4/icp5/icp6 payloads as ARGB on some systems.
Do not put PNG bytes there: the Finder thumbnail becomes colored stripes.
"""
from pathlib import Path
import struct
import sys

source = Path(sys.argv[1])
destination = Path(sys.argv[2])

def rle(channel):
    encoded = bytearray()
    offset = 0
    while offset < len(channel):
        run = 1
        while offset + run < len(channel) and run < 130 and channel[offset + run] == channel[offset]:
            run += 1
        if run >= 3:
            encoded.extend((run + 125, channel[offset]))
            offset += run
        else:
            start = offset
            offset += run
            while offset < len(channel) and offset - start < 128:
                if offset + 2 < len(channel) and channel[offset] == channel[offset+1] == channel[offset+2]:
                    break
                offset += 1
            encoded.append(offset - start - 1)
            encoded.extend(channel[start:offset])
    return bytes(encoded)

def chunk(kind, data):
    return kind + struct.pack('>I', len(data) + 8) + data

chunks = []
for rgb_type, mask_type, size in [(b'is32', b's8mk', 16), (b'il32', b'l8mk', 32), (b'ih32', b'h8mk', 48)]:
    rgba = (source / f'icon_{size}x{size}.rgba').read_bytes()
    if len(rgba) != size * size * 4:
        raise ValueError(f'invalid thumbnail: {size}')
    chunks.append(chunk(rgb_type, b''.join(rle(rgba[channel::4]) for channel in range(3))))
    chunks.append(chunk(mask_type, rgba[3::4]))

for kind, name, size in [
    (b'ic07', 'icon_128x128.png', 128),
    (b'ic08', 'icon_256x256.png', 256),
    (b'ic09', 'icon_512x512.png', 512),
    (b'ic10', 'icon_512x512@2x.png', 1024),
    (b'ic11', 'icon_16x16@2x.png', 32),
    (b'ic12', 'icon_32x32@2x.png', 64),
    (b'ic13', 'icon_128x128@2x.png', 256),
    (b'ic14', 'icon_256x256@2x.png', 512),
]:
    png = (source / name).read_bytes()
    if png[:8] != b'\x89PNG\r\n\x1a\n' or struct.unpack('>II', png[16:24]) != (size, size):
        raise ValueError(f'invalid icon dimensions: {name}')
    chunks.append(chunk(kind, png))
body = b''.join(chunks)
destination.write_bytes(b'icns' + struct.pack('>I', len(body) + 8) + body)
