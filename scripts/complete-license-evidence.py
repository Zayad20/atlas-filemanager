"""Preserve declarations when a published package omits a standalone license.

Stores package-authored metadata and any embedded notices, together with the
standard text of every declared SPDX alternative. Nonstandard licenses require
an explicit upstream notice override and are not silently treated as MIT.
"""
import hashlib
import importlib.util
import json
import re
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("notices", ROOT / "scripts/generate-third-party.py")
notices = importlib.util.module_from_spec(spec)
spec.loader.exec_module(notices)


def download(url):
    request = urllib.request.Request(url, headers={"User-Agent":"Atlas-license-audit/1.0"})
    with urllib.request.urlopen(request, timeout=30) as response:
        return response.read()


def main():
    inventory = json.loads((ROOT / "legal/dependency-inventory.json").read_text(encoding="utf-8"))
    missing = set(inventory["missingNotices"])
    path = ROOT / "legal/notice-overrides.json"
    overrides = json.loads(path.read_text())
    metadata = notices.cargo_metadata(all_features=True)
    packages = [("cargo", Path(p["manifest_path"]).parent, p) for p in metadata["packages"]]
    packages.extend(notices.web_packages())
    standard = ROOT / "legal/standard-licenses"
    standard.mkdir(parents=True, exist_ok=True)
    for ecosystem, directory, package in packages:
        name = package["name"]
        version = package["version"]
        if f"{ecosystem}: {name}@{version}" not in missing:
            continue
        expression = package.get("license")
        if not expression or "http" in str(expression):
            print(f"NEEDS UPSTREAM TEXT {ecosystem}:{name}@{version}")
            continue
        if isinstance(expression, dict):
            expression = expression["type"]
        tokens = [token for token in re.split(r"\s+|[()/]", expression) if token and token not in {"OR", "AND", "WITH"}]
        files = []
        for token in dict.fromkeys(tokens):
            target = standard / f"{token}.txt"
            url = f"https://raw.githubusercontent.com/spdx/license-list-data/v3.27.0/text/{token}.txt"
            if not target.is_file():
                target.write_bytes(download(url))
            files.append({"path":target.relative_to(ROOT).as_posix(), "url":url, "sha256":hashlib.sha256(target.read_bytes()).hexdigest()})
        folder = ROOT / "legal/upstream" / re.sub(r"[^a-zA-Z0-9._-]", "_", f"{ecosystem}:{name}@{version}")
        folder.mkdir(parents=True, exist_ok=True)
        attribution = {key: package.get(key) for key in ("name", "version", "license", "authors", "author", "contributors", "repository", "homepage") if package.get(key) is not None}
        body = ["UPSTREAM PACKAGE DECLARATION", json.dumps(attribution, ensure_ascii=False, indent=2), "",
                "This published package did not supply a standalone license file.",
                "Its upstream declaration is preserved above; standard license texts are attached.",
                "No copyright ownership or exception has been inferred.", ""]
        manifest = directory / ("Cargo.toml.orig" if ecosystem == "cargo" and (directory/"Cargo.toml.orig").is_file() else "Cargo.toml" if ecosystem == "cargo" else "package.json")
        body.append(f"Published manifest SHA-256: {hashlib.sha256(manifest.read_bytes()).hexdigest()}")
        # Keep supplied copyright/permission statements rather than inventing them.
        for source in sorted(directory.rglob("*")):
            relative = source.relative_to(directory)
            if not source.is_file() or any(part in {"node_modules", ".git", "target"} for part in relative.parts) or source.suffix.lower() not in {".md", ".rs", ".js", ".ts", ".txt"} or source.stat().st_size > 500000:
                continue
            content = notices.read(source)
            if "permission is hereby granted" in content.lower():
                start = content.lower().rfind("copyright", 0, content.lower().find("permission is hereby granted"))
                body.extend([f"\nEmbedded notice from {relative.as_posix()}:", content[max(0, start):][:7000]])
                break
            if source.name.lower().startswith("readme") and re.search(r"(?im)^#{1,3}\s+licen[sc]e", content):
                start = re.search(r"(?im)^#{1,3}\s+licen[sc]e", content).start()
                body.extend([f"\nLicense statement from {relative.as_posix()}:", content[start:][:7000]])
        target = folder / "UPSTREAM-DECLARATION.txt"
        target.write_text("\n".join(body) + "\n", encoding="utf-8")
        files.insert(0, {"path":target.relative_to(ROOT).as_posix(), "publishedManifestSha256":hashlib.sha256(manifest.read_bytes()).hexdigest(), "sha256":hashlib.sha256(target.read_bytes()).hexdigest()})
        overrides[f"{ecosystem}:{name}@{version}"] = {"basis":"Published package declaration and standard license texts; standalone upstream notice absent.", "notices":files}
        print(f"PRESERVED {ecosystem}:{name}@{version} ({expression})")
    path.write_text(json.dumps(overrides, indent=2, sort_keys=True)+"\n", encoding="utf-8")


if __name__ == "__main__":
    main()
