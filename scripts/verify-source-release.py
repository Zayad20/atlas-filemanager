"""Check that a prepared source snapshot still matches the working tree."""
import argparse
import hashlib
import json
from pathlib import Path

ROOT=Path(__file__).resolve().parents[1]


def verify(version):
    receipt=json.loads((ROOT/f"target/source/atlas-{version}-source.json").read_text(encoding="utf-8"))
    archive=ROOT/f"target/source/atlas-{version}-source.zip"
    if hashlib.sha256(archive.read_bytes()).hexdigest()!=receipt["sha256"]:
        raise SystemExit("Source archive hash mismatch")
    changed=[name for name,digest in receipt["workspaceHashes"].items() if not (ROOT/name).is_file() or hashlib.sha256((ROOT/name).read_bytes()).hexdigest()!=digest]
    if changed:
        raise SystemExit("Source snapshot is stale: "+", ".join(changed[:12]))
    print(f"Source snapshot matches {len(receipt['workspaceHashes'])} workspace inputs for {version}.")


if __name__=="__main__":
    parser=argparse.ArgumentParser(description=__doc__);parser.add_argument("--version",required=True)
    verify(parser.parse_args().version)
