"""Fetch missing upstream notices at the source revision recorded by Cargo/npm.

This helper writes reviewable evidence to legal/upstream and notice-overrides.json.
It never chooses a new license or silently falls back to the latest upstream code.
"""
from __future__ import annotations

import hashlib
import json
import re
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

from importlib.util import module_from_spec, spec_from_file_location

ROOT = Path(__file__).resolve().parents[1]
spec = spec_from_file_location("notices", ROOT / "scripts/generate-third-party.py")
notices = module_from_spec(spec)
spec.loader.exec_module(notices)
CACHE = ROOT / "target/license-fetch-cache"
CACHE.mkdir(parents=True, exist_ok=True)


def get(url: str) -> bytes:
    cache = CACHE / hashlib.sha256(url.encode()).hexdigest()
    if cache.is_file():
        return cache.read_bytes()
    request = urllib.request.Request(url, headers={"User-Agent": "Atlas-license-audit/1.0", "Accept": "application/json" if "registry.npmjs.org" in url else "*/*"})
    with urllib.request.urlopen(request, timeout=30) as response:
        value = response.read()
    cache.write_bytes(value)
    return value


def github_repo(url: str | dict | None) -> str | None:
    if isinstance(url, dict):
        url = url.get("url")
    match = re.search(r"github\.com[:/]([^/]+/[^/#]+)", url or "")
    return match.group(1).removesuffix(".git") if match else None


def fetch_entry(item: tuple[str, dict, str, str, str]) -> tuple[str, dict] | None:
    key, package, repo, revision, subdir = item
    try:
        tree = json.loads(get(f"https://api.github.com/repos/{repo}/git/trees/{revision}?recursive=1"))
        ancestors = {"."}
        ancestors.update(str(path).replace("\\", "/") for path in Path(subdir).parents)
        if subdir:
            ancestors.add(subdir)
        paths = [entry["path"] for entry in tree.get("tree", [])
                 if entry["type"] == "blob" and notices.NOTICE_NAME.match(Path(entry["path"]).name)
                 and str(Path(entry["path"]).parent).replace("\\", "/") in ancestors]
        if not paths:
            print(f"NO UPSTREAM NOTICE {key} ({repo}@{revision})", flush=True)
            return None
        saved = []
        folder = ROOT / "legal/upstream" / re.sub(r"[^a-zA-Z0-9._-]", "_", key)
        folder.mkdir(parents=True, exist_ok=True)
        for path in paths:
            url = f"https://raw.githubusercontent.com/{repo}/{revision}/{path}"
            content = get(url)
            target = folder / path.replace("/", "__")
            target.write_bytes(content)
            saved.append({"path": target.relative_to(ROOT).as_posix(), "url": url,
                          "sha256": hashlib.sha256(content).hexdigest()})
        print(f"FETCHED {key}: {len(saved)} notices", flush=True)
        return key, {"revision": revision, "repository": f"https://github.com/{repo}", "notices": saved}
    except Exception as error:
        print(f"FETCH FAILED {key}: {error}", flush=True)
        return None


def main() -> None:
    metadata = notices.cargo_metadata(all_features=True)
    inventory = json.loads((ROOT / "legal/dependency-inventory.json").read_text(encoding="utf-8"))
    missing = set(inventory["missingNotices"])
    jobs = []
    for package in metadata["packages"]:
        key = f"cargo:{package['name']}@{package['version']}"
        if f"cargo: {package['name']}@{package['version']}" not in missing:
            continue
        directory = Path(package["manifest_path"]).parent
        vcs = directory / ".cargo_vcs_info.json"
        repo = github_repo(package.get("repository"))
        if repo and vcs.is_file():
            info = json.loads(vcs.read_text())
            jobs.append((key, package, repo, info["git"]["sha1"], info.get("path_in_vcs", "")))
        else:
            print(f"MANUAL REVIEW {key}: no pinned upstream revision", flush=True)
    for site, directory, package in notices.web_packages():
        key = f"{site}:{package['name']}@{package['version']}"
        if f"{site}: {package['name']}@{package['version']}" not in missing:
            continue
        try:
            url = f"https://registry.npmjs.org/{urllib.parse.quote(package['name'], safe='')}/{package['version']}"
            published = json.loads(get(url))
            repo = github_repo(published.get("repository") or package.get("repository"))
            revision = published.get("gitHead")
            subdir = (published.get("repository") or {}).get("directory", "") if isinstance(published.get("repository"), dict) else ""
            if repo and revision:
                jobs.append((key, package, repo, revision, subdir))
            else:
                print(f"MANUAL REVIEW {key}: no pinned upstream revision", flush=True)
        except Exception as error:
            print(f"METADATA FAILED {key}: {error}", flush=True)
    path = ROOT / "legal/notice-overrides.json"
    overrides = json.loads(path.read_text()) if path.is_file() else {}
    with ThreadPoolExecutor(max_workers=5) as executor:
        for result in executor.map(fetch_entry, jobs):
            if result:
                overrides[result[0]] = result[1]
    path.write_text(json.dumps(overrides, indent=2, sort_keys=True) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
