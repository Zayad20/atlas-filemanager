"""Run the native regression with stderr disconnected after startup.

Build first: cargo build --features ui-lab,perf-log --example miller_conversion_diagnostic
"""
import argparse
import json
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/examples/miller_conversion_diagnostic.exe"))
    parser.add_argument("--output", type=Path, default=Path("artifacts/miller-conversion-closed-stderr"))
    args = parser.parse_args()
    process = subprocess.Popen([str(args.binary.resolve()), str(args.output.resolve())], stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    try:
        # Keep stderr alive for startup's ordinary theme announcement. Rendering
        # and conversions must survive losing the performance log destination.
        for line in process.stderr:
            if b"themes registered" in line:
                break
        else:
            raise RuntimeError("The native app did not reach startup")
        process.stderr.close()
        code = process.wait(timeout=120)
        if code:
            raise RuntimeError(f"Native diagnostic exited {code} after stderr was closed")
        checks = json.loads((args.output / "passed.json").read_text(encoding="utf-8"))
        report = {"closed_stderr": True, "exit_code": code, "checks": checks}
        (args.output / "closed-stderr.json").write_text(json.dumps(report, ensure_ascii=False, indent=2), encoding="utf-8")
        print(f"Closed stderr: process exited {code}; {len(checks)} native checks passed")
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()


if __name__ == "__main__":
    main()
