"""Launch only the supplied Atlas build with isolated preferences and sample idle CPU.

This is a process smoke check, not an interactive or visual acceptance test.
Requires psutil. Artifacts stay in the explicitly selected output directory.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import time

import psutil


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--settle", type=float, default=8.0)
    parser.add_argument("--sample", type=float, default=10.0)
    args = parser.parse_args()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    config = output / "config"
    files = output / "files"
    config.mkdir()
    files.mkdir()
    for index in range(32):
        (files / f"file{index}.txt").write_text("Atlas parity smoke fixture\n", encoding="utf-8")
    (config / "settings.json").write_text(json.dumps({
        "getting_started_seen": True, "autosave_session": False,
        "show_inspector": False, "eco_mode": False,
    }), encoding="utf-8")
    environment = dict(os.environ, ATLAS_CONFIG_DIR=str(config))
    startup = subprocess.STARTUPINFO() if os.name == "nt" else None
    if startup:
        startup.dwFlags |= subprocess.STARTF_USESHOWWINDOW
        startup.wShowWindow = 0
    started = time.monotonic()
    with (output / "runtime.log").open("wb") as log:
        process = subprocess.Popen([str(args.exe.resolve()), str(files)],
            env=environment, stdin=subprocess.DEVNULL, stdout=log, stderr=log,
            startupinfo=startup)
        try:
            time.sleep(args.settle)
            if process.poll() is not None:
                raise RuntimeError(f"Atlas exited during startup: {process.returncode}; see runtime.log")
            observed = psutil.Process(process.pid)
            before = observed.cpu_times()
            sample_start = time.monotonic()
            time.sleep(args.sample)
            after = observed.cpu_times()
            seconds = time.monotonic() - sample_start
            cpu_seconds = after.user + after.system - before.user - before.system
            report = {
                "executable": str(args.exe.resolve()), "pid": process.pid,
                "settle_seconds": args.settle, "sample_seconds": seconds,
                "cpu_seconds": cpu_seconds,
                "cpu_percent_one_core": 100.0 * cpu_seconds / seconds,
                "cpu_percent_machine": 100.0 * cpu_seconds / seconds / (psutil.cpu_count() or 1),
                "working_set_bytes": observed.memory_info().rss,
                "total_seconds": time.monotonic() - started,
                "scope": "isolated 32-file directory; no inspector; eco mode off; startup requested hidden",
                "visual_acceptance": "not performed",
            }
            (output / "idle.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
            print(json.dumps(report, indent=2))
        finally:
            # Only the process created here is stopped; no other Atlas windows are touched.
            if process.poll() is None:
                process.terminate()
                process.wait(timeout=15)


if __name__ == "__main__":
    main()
