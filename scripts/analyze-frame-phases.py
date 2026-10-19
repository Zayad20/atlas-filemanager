"""Summarize GPUI draw phases and Atlas Details spans without summing nested spans.

Reads only the supplied capture. Old captures remain readable, but their
prepaint/layout split cannot be recovered retroactively.
"""
import argparse
import json
import math
from pathlib import Path
import re
import statistics


BUCKETS = (
    "root_request_other", "layout_solve", "prepaint_other", "hit_test",
    "paint", "a11y_finish",
)
SUBMETRICS = ("layout_measure", "layout_solve_ex_measure")
TIMING = re.compile(r"([\w.]+)=(-?\d+(?:\.\d+)?)ms\b")
DETAIL_TIMING = re.compile(r"(details\.[\w.]+)_ms=(\d+(?:\.\d+)?)\b")


def stats(values):
    values = sorted(values)
    if not values:
        return None
    return {
        "n": len(values),
        "median_ms": round(statistics.median(values), 3),
        "p95_ms": values[max(0, math.ceil(len(values) * 0.95) - 1)],
        "max_ms": values[-1],
    }


def analyze(lines):
    frames, details, issues = [], [], []
    pending = None
    for line_number, line in enumerate(lines, 1):
        if "[gpui-frame]" in line:
            pending = {"line": line_number, **{
                key: float(value) for key, value in TIMING.findall(line)
            }}
            if all(key in pending for key in ("total", "prepaint", "paint")):
                frames.append(pending)
            else:
                issues.append(f"line {line_number}: incomplete gpui-frame")
                pending = None
        elif "[gpui-phases]" in line:
            phases = {key: float(value) for key, value in TIMING.findall(line)}
            if pending is None or "phases" in pending:
                issues.append(f"line {line_number}: phases without a unique preceding draw")
            elif not all(key in phases for key in BUCKETS):
                issues.append(f"line {line_number}: incomplete gpui-phases")
            elif any(value < 0 for value in phases.values()):
                issues.append(f"line {line_number}: negative phase duration")
            else:
                pending["phases"] = phases
                pending["phase_sum_ms"] = round(sum(phases[key] for key in BUCKETS), 3)
                # Six buckets rounded to .001ms, total rounded to .01ms;
                # allow timer boundaries as well. Never add the submetrics.
                if abs(pending["phase_sum_ms"] - pending["total"]) > 0.05:
                    issues.append(f"line {line_number}: draw buckets differ from total by >0.05ms")
                if phases.get("layout_measure", 0) > phases["layout_solve"] + 0.002:
                    issues.append(f"line {line_number}: leaf measurement exceeds inclusive solver")
        if "frame_summary " in line:
            detail = {key: float(value) for key, value in DETAIL_TIMING.findall(line)}
            if detail:
                details.append({"line": line_number, "timings": detail})

    split_frames = [frame for frame in frames if "phases" in frame]
    detail_keys = sorted({key for sample in details for key in sample["timings"]})
    return {
        "draws": len(frames),
        "draws_with_phase_split": len(split_frames),
        "draw_total": stats(frame["total"] for frame in frames),
        "disjoint_draw_buckets": {
            key: stats(frame["phases"][key] for frame in split_frames)
            for key in BUCKETS
        },
        "solver_submetrics_do_not_add_to_buckets": {
            key: stats(frame["phases"][key] for frame in split_frames if key in frame["phases"])
            for key in SUBMETRICS
        },
        "details_app_intervals_inclusive_spans": {
            key: stats(sample["timings"].get(key, 0.0) for sample in details)
            for key in detail_keys
        },
        "worst_draws": sorted(frames, key=lambda frame: frame["total"], reverse=True)[:10],
        "issues": issues,
        "notes": [
            "CPU wall time, not GPU timestamps or displayed FPS. Profiling adds overhead.",
            "layout_solve includes leaf measurement; layout_measure and layout_solve_ex_measure subdivide it.",
            "root_request_other excludes solver time within the main root request; later requests remain in prepaint_other.",
            "Details spans are inclusive sums between App renders, not necessarily one GPUI draw. Do not add parents and children.",
            "Details statistics use 0 for a missing metric in a Details-bearing summary; absent Details summaries are excluded.",
            "Older captures without gpui-phases cannot separate layout from prepaint retroactively.",
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("capture", type=Path)
    parser.add_argument("--output", type=Path, help="Optional JSON output path")
    args = parser.parse_args()
    with args.capture.open(encoding="utf-8", errors="replace") as stream:
        result = {"capture": str(args.capture), **analyze(stream)}
    rendered = json.dumps(result, indent=2, ensure_ascii=False)
    if args.output:
        args.output.write_text(rendered + "\n", encoding="utf-8")
    print(rendered)


if __name__ == "__main__":
    main()
