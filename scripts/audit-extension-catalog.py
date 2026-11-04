"""Compare bundled extension manifests with a downloaded marketplace registry.

Usage: python scripts/audit-extension-catalog.py --registry registry.json --output audit.json
This reads local files only; it never installs or publishes packages.
"""
import argparse
import json
from pathlib import Path
import tomllib


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--registry", type=Path, required=True)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    registry = json.loads(args.registry.read_text(encoding="utf-8"))
    remote = registry["extensions"]
    rows = []
    for path in sorted((root / "extensions").glob("*/extension.toml")):
        manifest = tomllib.loads(path.read_text(encoding="utf-8"))
        published = remote.get(manifest["id"])
        differences = []
        if published:
            for field, remote_field in [("name", "name"), ("description", "description"), ("version", "latest_version")]:
                if manifest.get(field, "") != published.get(remote_field, ""):
                    differences.append(field)
        rows.append({
            "id": manifest["id"], "name": manifest["name"], "description": manifest.get("description", ""),
            "version": manifest["version"], "kind": manifest["kind"], "hidden": manifest.get("hidden", False),
            "remote_version": published["latest_version"] if published else None,
            "remote_differences": differences,
        })
    report = {
        "registry_last_updated": registry.get("last_updated"),
        "bundled_manifests": len(rows),
        "feature_extensions": sum(row["kind"] != "tool_provider" for row in rows),
        "tool_providers": sum(row["kind"] == "tool_provider" for row in rows),
        "visible_installed": sum(not row["hidden"] for row in rows),
        "marketplace_entries": len(remote),
        "bundled_not_in_marketplace": [row["id"] for row in rows if row["remote_version"] is None],
        "extensions": rows,
    }
    output = json.dumps(report, ensure_ascii=False, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(output, encoding="utf-8")
    print(output)


if __name__ == "__main__":
    main()
