"""Verify actual native resize frames, including the first one at each size.

Build resize_diagnostic with ui-lab before running. Results use a fresh isolated
configuration and retain the original investigation captures for comparison.
"""
from pathlib import Path
import argparse
import json
import os
import subprocess

ROOT = Path(__file__).resolve().parents[1]
SCENARIOS = {
    "no-animations": {},
    "no-composition": {"GPUI_DISABLE_DIRECT_COMPOSITION": "1"},
    "animations": {"RESIZE_PROBE_ANIMATIONS": "1"},
    "split": {"RESIZE_PROBE_SPLIT": "1"},
}
TARGETS = [(1480, 800), (1080, 800), (1080, 960), (1080, 600)]


def geometry(frame):
    return sorted(set(tuple(rect) for rect in frame["pane_rects"])), frame["dividers"]


def verify_trace(trace, split):
    checks = []
    for target in TARGETS:
        frames = [frame for frame in trace if tuple(frame["viewport"]) == target]
        assert frames, f"no frames for viewport {target}"
        final_geometry = geometry(frames[-1])
        assert final_geometry[0], f"no painted panes for {target}"
        assert len(final_geometry[0]) == (2 if split else 1), (target, final_geometry[0])
        assert final_geometry[1], f"Details header missing for {target}"
        right = max(x + width for x, y, width, height in final_geometry[0])
        bottom = max(y + height for x, y, width, height in final_geometry[0])
        assert abs(right - (target[0] - 3)) < 0.1, ("wrong right edge", target, right)
        assert abs(bottom - (target[1] - 3)) < 0.1, ("wrong bottom edge", target, bottom)
        for index, frame in enumerate(frames):
            assert geometry(frame) == final_geometry, (
                "resize geometry trails viewport", target, index,
                geometry(frame), final_geometry,
            )
        checks.append({"viewport": target, "frames": len(frames), "first_frame_correct": True})
    return checks


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scenario", choices=SCENARIOS)
    parser.add_argument("--output", type=Path, default=ROOT / "artifacts/resize-fix-20261020")
    args = parser.parse_args()
    exe = ROOT / "target/release-perf/examples/resize_diagnostic.exe"
    results = {}
    for name, overrides in SCENARIOS.items():
        if args.scenario and name != args.scenario:
            continue
        directory = args.output.resolve() / name
        directory.mkdir(parents=True, exist_ok=True)
        env = os.environ.copy()
        for key in ["GPUI_DISABLE_DIRECT_COMPOSITION", "RESIZE_PROBE_ANIMATIONS", "RESIZE_PROBE_SPLIT"]:
            env.pop(key, None)
        env.update(ATLAS_PERF="1", ATLAS_PERF_EVENTS="0", GPUI_FRAME_LOG="0", RUST_BACKTRACE="1")
        env.update(overrides)
        failure = directory / "failed.txt"
        failure.unlink(missing_ok=True)
        with (directory / "stdout.log").open("w", encoding="utf-8") as stdout, (directory / "stderr.log").open("w", encoding="utf-8") as stderr:
            result = subprocess.run([str(exe), str(directory)], cwd=ROOT, env=env, stdout=stdout, stderr=stderr, timeout=60)
        assert result.returncode == 0, (name, result.returncode, str(directory / "stderr.log"))
        assert not failure.exists(), failure.read_text() if failure.exists() else ""
        trace = json.loads((directory / "frame-trace.json").read_text())
        checks = verify_trace(trace, name == "split")
        results[name] = checks
        print(f"{name}: PASS — first frame correct at all {len(checks)} sizes ({sum(c['frames'] for c in checks)} frames checked)")
    (args.output / "verification.json").write_text(json.dumps(results, indent=2), encoding="utf-8")


if __name__ == "__main__":
    main()
