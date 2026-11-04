"""Recover separately licensed PNG build inputs from a matching Atlas binary.

Usage: python scripts/recover-licensed-artwork.py PATH_TO_ATLAS_EXE_OR_MSIX
The images remain subject to Icons8's terms; this is not an icon redistribution.
"""
import hashlib
import json
import struct
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def recover(binary: bytes, assets: list[dict]) -> dict[str, bytes]:
    expected = {item["sha256"]: item["path"] for asset in assets
                if asset["name"] == "Icons8 Liquid Glass" for item in asset.get("files", [])}
    found = {}
    position = 0
    while len(found) < len(expected):
        start = binary.find(b"\x89PNG\r\n\x1a\n", position)
        if start < 0:
            break
        position = start + 8
        cursor = position
        while cursor + 12 <= len(binary):
            length = struct.unpack_from(">I", binary, cursor)[0]
            kind = binary[cursor + 4:cursor + 8]
            end = cursor + 12 + length
            if length > 16 * 1024 * 1024 or end > len(binary):
                break
            cursor = end
            if kind == b"IEND":
                image = binary[start:end]
                digest = hashlib.sha256(image).hexdigest()
                if digest in expected:
                    found[expected[digest]] = image
                position = end
                break
    missing = set(expected.values()) - set(found)
    if missing:
        raise ValueError("The binary does not contain this release's exact artwork: " + ", ".join(sorted(missing)))
    return found


def main():
    if len(sys.argv) != 2:
        raise SystemExit(__doc__)
    source = Path(sys.argv[1])
    if zipfile.is_zipfile(source):
        with zipfile.ZipFile(source) as archive:
            binary = archive.read("atlas.exe")
    else:
        binary = source.read_bytes()
    assets = json.loads((ROOT / "legal/assets.json").read_text(encoding="utf-8"))
    found = recover(binary, assets)
    for relative, content in found.items():
        target = ROOT / relative
        if not target.resolve().is_relative_to(ROOT.resolve()):
            raise ValueError("Invalid artwork destination")
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(content)
    print(f"Recovered {len(found)} exact PNG inputs. Their Icons8 license remains applicable.")


if __name__ == "__main__":
    main()
