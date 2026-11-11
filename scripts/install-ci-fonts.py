"""Install verified CJK fonts for rendering tests, never for application packaging."""

import argparse
import hashlib
import os
from pathlib import Path
import sys
import tempfile
from urllib.request import urlopen


COMMIT = "f8d157532fbfaeda587e826d4cd5b21a49186f7c"
FONTS = (
    ("Sans", "NotoSansCJKsc-Regular.otf", "2c76254f6fc379fddfce0a7e84fb5385bb135d3e399294f6eeb6680d0365b74b"),
    ("Serif", "NotoSerifCJKsc-Regular.otf", "2a2eae2628df83556c54018c41e20fa532c1b862c5256ae8b3f23feb918d12ca"),
)


def default_directory():
    if sys.platform == "win32":
        return Path(os.environ["LOCALAPPDATA"]) / "Microsoft/Windows/Fonts"
    if sys.platform == "darwin":
        return Path.home() / "Library/Fonts"
    raise SystemExit("This test environment supports macOS and Windows.")


def file_digest(path):
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(64 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def install(directory):
    directory.mkdir(parents=True, exist_ok=True)
    for kind, filename, expected in FONTS:
        destination = directory / filename
        if destination.is_file() and file_digest(destination) == expected:
            print(f"Verified {filename}")
            continue
        url = (
            f"https://raw.githubusercontent.com/notofonts/noto-cjk/{COMMIT}"
            f"/{kind}/OTF/SimplifiedChinese/{filename}"
        )
        # Verify before replacing; failures leave any existing font intact.
        with tempfile.NamedTemporaryFile(dir=directory, delete=False) as output:
            temporary = Path(output.name)
        try:
            with urlopen(url, timeout=60) as response, temporary.open("wb") as output:
                while chunk := response.read(64 * 1024):
                    output.write(chunk)
            if file_digest(temporary) != expected:
                raise ValueError(f"Unexpected font checksum: {filename}")
            os.replace(temporary, destination)
            print(f"Installed {filename}")
        finally:
            temporary.unlink(missing_ok=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, help="override the user font directory")
    args = parser.parse_args()
    install(args.directory if args.directory is not None else default_directory())
