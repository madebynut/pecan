"""Read Cargo's resolved application version; validate release tags before building."""
import json
import os
from pathlib import Path
import subprocess

root = Path(__file__).resolve().parent.parent
metadata = json.loads(subprocess.check_output(
    ["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], cwd=root
))
version = next(package["version"] for package in metadata["packages"] if package["name"] == "pecan-app")
if os.environ.get("GITHUB_REF_TYPE") == "tag":
    tag = os.environ["GITHUB_REF_NAME"]
    if (tag.startswith("v") or tag[:1].isdigit()) and tag not in (version, f"v{version}"):
        raise SystemExit(f"Tag {tag!r} must match Cargo version {version!r}")
print(version)
