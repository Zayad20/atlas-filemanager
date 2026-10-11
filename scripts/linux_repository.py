#!/usr/bin/env python3
"""Create signed APT, RPM and pacman repositories in a static hosting directory."""
import argparse
import gzip
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import xml.etree.ElementTree as ET

from linux_release import sha256, validate_version


def run(*args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def verify_assets(directory):
    for package in directory.rglob("*"):
        if not package.is_file() or not package.name.endswith((".deb", ".rpm", ".pkg.tar.zst")):
            continue
        checksum = package.with_name(package.name + ".sha256")
        if not checksum.exists() or checksum.read_text().split()[0] != sha256(package):
            raise ValueError(f"Missing or invalid checksum for {package.name}")


def build(artifacts, output, base_url, fingerprint):
    if not re.fullmatch(r"https://[A-Za-z0-9.-]+(?::\d+)?(?:/[A-Za-z0-9._~/-]*)?", base_url):
        raise ValueError("Repository URL must be HTTPS without query, fragment or whitespace")
    fingerprint = fingerprint.upper()
    if not re.fullmatch(r"(?:[A-F0-9]{40}|[A-F0-9]{64})", fingerprint):
        raise ValueError("Set the full signing fingerprint")
    verify_assets(artifacts)
    info = json.loads((artifacts / "atlas-linux.json").read_text())
    validate_version(info["version"])
    if info["prerelease"] or "-" in info["version"]:
        raise ValueError("Prereleases must not enter the stable repositories")
    output.mkdir(parents=True, exist_ok=False)
    key = run("gpg", "--batch", "--armor", "--export", fingerprint, stdout=subprocess.PIPE).stdout
    if not key:
        raise ValueError("Signing key is unavailable")
    (output / "atlas-signing-key.asc").write_bytes(key)
    (output / "signing-fingerprint.txt").write_text(fingerprint + "\n")
    passphrase = os.environ.get("ATLAS_GPG_PASSPHRASE", "")

    def sign(path, clearsign=False):
        target = path.with_name("InRelease") if clearsign else path.with_name(path.name + ".asc")
        run("gpg", "--batch", "--yes", "--pinentry-mode", "loopback", "--passphrase-fd", "0",
            "--local-user", fingerprint, "--armor", "--output", target,
            "--clearsign" if clearsign else "--detach-sign", path, input=passphrase.encode())
        return target

    apt = output / "apt"
    pool = apt / "pool/main/a/atlas"
    rpm = output / "rpm/x86_64"
    arch = output / "pacman/x86_64"
    for directory in [pool, rpm, arch]:
        directory.mkdir(parents=True)
    for pattern, destination in [("*.deb", pool), ("*.rpm", rpm), ("*.pkg.tar.zst", arch)]:
        packages = list(artifacts.rglob(pattern))
        if not packages:
            raise ValueError(f"No packages matching {pattern}")
        for package in packages:
            shutil.copy2(package, destination / package.name)
    index_dir = apt / "dists/stable/main/binary-amd64"
    index_dir.mkdir(parents=True)
    index = run("apt-ftparchive", "packages", "pool", cwd=apt, stdout=subprocess.PIPE).stdout
    (index_dir / "Packages").write_bytes(index)
    (index_dir / "Packages.gz").write_bytes(gzip.compress(index, mtime=0))
    release_dir = apt / "dists/stable"
    release_data = run("apt-ftparchive", "-o", "APT::FTPArchive::Release::Origin=Atlas",
        "-o", "APT::FTPArchive::Release::Label=Atlas", "-o", "APT::FTPArchive::Release::Suite=stable",
        "-o", "APT::FTPArchive::Release::Codename=stable", "-o", "APT::FTPArchive::Release::Architectures=amd64",
        "-o", "APT::FTPArchive::Release::Components=main", "release", "dists/stable", cwd=apt,
        stdout=subprocess.PIPE).stdout
    release_path = release_dir / "Release"
    release_path.write_bytes(release_data)
    sign(release_path, clearsign=True)
    sign(release_path).rename(release_dir / "Release.gpg")
    # Package signatures are required by RPM clients in addition to repo metadata.
    with tempfile.NamedTemporaryFile(mode="w", prefix="atlas-passphrase-") as secret:
        secret.write(passphrase)
        secret.flush()
        for package in rpm.glob("*.rpm"):
            run("rpmsign", "--define", f"_gpg_name {fingerprint}",
                "--define", f"__gpg {shutil.which('gpg')}",
                "--define", f"_gpg_path {os.environ['GNUPGHOME']}",
                "--define", f"_gpg_sign_cmd_extra_args --pinentry-mode loopback --passphrase-file {secret.name}",
                "--addsign", package)
    run("createrepo_c", rpm)
    sign(rpm / "repodata/repomd.xml")
    for package in arch.glob("*.pkg.tar.zst"):
        signature = sign(package)
        # pacman expects a binary detached signature named .sig.
        run("gpg", "--batch", "--dearmor", "--output", str(package) + ".sig", signature)
        signature.unlink()
    current_arch = list(artifacts.glob("*.pkg.tar.zst"))
    if len(current_arch) != 1:
        raise ValueError("Expected one current Arch package")
    run("repo-add", arch / "atlas.db.tar.gz", arch / current_arch[0].name)
    for name in ["atlas.db.tar.gz", "atlas.files.tar.gz"]:
        signature = sign(arch / name)
        run("gpg", "--batch", "--dearmor", "--output", str(arch / name) + ".sig", signature)
        signature.unlink()
    # Pages artifacts reject symbolic links. Materialize repo-add's aliases.
    for path in arch.iterdir():
        if path.is_symlink():
            content = path.read_bytes()
            path.unlink()
            path.write_bytes(content)
    for name in ["atlas.db", "atlas.files"]:
        shutil.copy2(arch / (name + ".tar.gz.sig"), arch / (name + ".sig"))
    installer = (artifacts / "install-linux.sh").read_text()
    installer = installer.replace("DEFAULT_REPOSITORY_URL=''", f"DEFAULT_REPOSITORY_URL='{base_url.rstrip('/')}'")
    installer = installer.replace("DEFAULT_SIGNING_FINGERPRINT=''", f"DEFAULT_SIGNING_FINGERPRINT='{fingerprint}'")
    (output / "install-linux.sh").write_text(installer)
    shutil.copy2(artifacts / "atlas-linux.json", output / "latest.json")
    shutil.copy2(artifacts / "release-notes.md", output / "release-notes.md")
    # Publish portable-artifact checksums under the same signing identity.
    shutil.copy2(artifacts / "SHA256SUMS", output / "release-SHA256SUMS")
    sign(output / "release-SHA256SUMS")
    feed = ET.Element("feed", xmlns="http://www.w3.org/2005/Atom")
    timestamp = info["date"] + "T00:00:00Z"
    for tag, value in [("id", base_url), ("title", "Atlas releases"), ("updated", timestamp)]:
        ET.SubElement(feed, tag).text = value
    ET.SubElement(feed, "link", href=base_url.rstrip("/") + "/releases.atom", rel="self")
    entry = ET.SubElement(feed, "entry")
    for tag, value in [("id", info["release_url"]), ("title", "Atlas " + info["version"]), ("updated", timestamp)]:
        ET.SubElement(entry, tag).text = value
    ET.SubElement(entry, "link", href=info["release_url"])
    ET.SubElement(entry, "content", type="text").text = info["notes"]
    ET.SubElement(ET.SubElement(entry, "author"), "name").text = "Atlas contributors"
    ET.indent(feed)
    ET.ElementTree(feed).write(output / "releases.atom", encoding="utf-8", xml_declaration=True)
    (output / "index.html").write_text('<!doctype html><meta charset="utf-8"><title>Atlas Linux repository</title><h1>Atlas Linux repository</h1><p><a href="release-notes.md">Release notes</a> · <a href="releases.atom">Announcements</a> · <a href="install-linux.sh">Native installer</a></p>')
    files = sorted(path for path in output.rglob("*") if path.is_file())
    (output / "SHA256SUMS").write_text("".join(f"{sha256(path)}  {path.relative_to(output).as_posix()}\n" for path in files))
    sign(output / "SHA256SUMS")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--url", required=True)
    parser.add_argument("--fingerprint", required=True)
    args = parser.parse_args()
    build(args.artifacts.resolve(), args.output.resolve(), args.url, args.fingerprint)
