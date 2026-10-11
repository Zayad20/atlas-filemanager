#!/usr/bin/env python3
"""Run the CI native installation/GUI matrix locally, without publishing anything."""
import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
from datetime import datetime, timezone
import json
from pathlib import Path
import shutil
import subprocess
import threading
import time
import uuid

from linux_release import ROOT, sha256, validate_version


def verify_artifacts(directory):
    verified = set()
    for line in (directory / "SHA256SUMS").read_text().splitlines():
        digest, filename = line.split(maxsplit=1)
        path = directory / filename.lstrip("*")
        if path.resolve().parent != directory.resolve() or sha256(path) != digest:
            raise ValueError(f"Invalid artifact checksum: {filename}")
        verified.add(path.name)
    required = {path.name for path in directory.iterdir() if path.is_file()
                and path.name != "SHA256SUMS" and not path.name.endswith((".sha256", ".asc"))}
    if required - verified:
        raise ValueError("Checksums do not cover: " + ", ".join(sorted(required - verified)))
    info = json.loads((directory / "atlas-linux.json").read_text())
    validate_version(info["version"])
    if info["arch"] != "x86_64":
        raise ValueError("This matrix currently supports x86_64")
    return info["version"]


def engine_available(engine):
    if not shutil.which(engine):
        return False
    try:
        return subprocess.run([engine, "info"], stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL, timeout=15).returncode == 0
    except subprocess.TimeoutExpired:
        return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, default=ROOT / "target/linux-dist")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--engine", choices=["auto", "podman", "docker"], default="auto")
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--only", nargs="+", help="Matrix IDs, for example ubuntu-24 debian-13")
    args = parser.parse_args()
    if not 1 <= args.jobs <= 4:
        parser.error("Use between 1 and 4 concurrent containers")
    artifacts = args.artifacts.resolve()
    version = verify_artifacts(artifacts)
    matrix = json.loads((ROOT / "packaging/linux/test-matrix.json").read_text())["include"]
    if args.only:
        unknown = set(args.only) - {row["id"] for row in matrix}
        if unknown:
            parser.error("Unknown matrix IDs: " + ", ".join(sorted(unknown)))
        matrix = [row for row in matrix if row["id"] in args.only]
    candidates = ["podman", "docker"] if args.engine == "auto" else [args.engine]
    engine = next((name for name in candidates if engine_available(name)), None)
    if engine is None:
        parser.error("Start Docker or configure rootless Podman first")
    rootless = engine == "podman" and subprocess.check_output(
        [engine, "info", "--format", "{{.Host.Security.Rootless}}"], text=True).strip() == "true"
    timestamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    output = (args.output or ROOT / "target/linux-local-tests" / timestamp).resolve()
    output.mkdir(parents=True, exist_ok=False)
    run_id = uuid.uuid4().hex[:8]
    names = [f"atlas-local-{run_id}-{row['id']}" for row in matrix]
    lock = threading.Lock()
    stopping = threading.Event()
    active = {}
    print(f"Testing Atlas {version}: {len(matrix)} distros using {engine}. Logs: {output}", flush=True)

    def test(row):
        logs = output / row["id"]
        logs.mkdir(mode=0o777)
        logs.chmod(0o777)
        name = f"atlas-local-{run_id}-{row['id']}"
        command = [engine, "run", "--rm", "--name", name, "--security-opt", "seccomp=unconfined"]
        if rootless:
            command += ["--userns=keep-id:uid=0,gid=0"]
        command += ["-e", f"ATLAS_VERSION={version}",
                    "-v", f"{artifacts}:/artifacts:ro", "-v", f"{ROOT / 'scripts'}:/scripts:ro",
                    "-v", f"{logs}:/logs", row["image"], "bash", "/scripts/linux-container-test.sh", row["format"]]
        started = time.monotonic()
        with lock:
            print(f"START {row['name']}", flush=True)
        with (logs / "commands.log").open("w") as log:
            log.write(json.dumps(command) + "\n")
            log.flush()
            with lock:
                if stopping.is_set():
                    return dict(row, exit_code=130, seconds=0)
                process = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT)
                active[name] = process
            try:
                status = process.wait(timeout=900)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait()
                subprocess.run([engine, "rm", "--force", name], stdout=log, stderr=subprocess.STDOUT)
                status = 124
            finally:
                with lock:
                    active.pop(name, None)
        result = dict(row, exit_code=status, seconds=round(time.monotonic() - started, 1))
        with lock:
            print(f"{'PASS' if status == 0 else 'FAIL'} {row['name']} ({result['seconds']}s)", flush=True)
        return result

    results = []
    executor = ThreadPoolExecutor(max_workers=args.jobs)
    futures = [executor.submit(test, row) for row in matrix]
    try:
        for future in as_completed(futures):
            results.append(future.result())
            report = dict(version=version, engine=engine, artifact_checksums_sha256=sha256(artifacts / "SHA256SUMS"),
                          results=sorted(results, key=lambda row: row["id"]))
            (output / "results.json").write_text(json.dumps(report, indent=2) + "\n")
    except BaseException:
        stopping.set()
        for future in futures:
            future.cancel()
        with lock:
            for process in active.values():
                process.terminate()
        subprocess.run([engine, "rm", "--force", *names], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        raise
    finally:
        executor.shutdown(wait=True, cancel_futures=True)
        # These names belong only to this run; never clean up unrelated containers.
        subprocess.run([engine, "rm", "--force", *names], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    failed = [row["name"] for row in results if row["exit_code"]]
    print(f"Result: {len(results) - len(failed)}/{len(matrix)} passed. Report: {output / 'results.json'}", flush=True)
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
