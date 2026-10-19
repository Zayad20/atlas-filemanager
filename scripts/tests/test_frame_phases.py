import importlib.util
from pathlib import Path
import unittest


SPEC = importlib.util.spec_from_file_location(
    "frame_phases", Path(__file__).resolve().parents[1] / "analyze-frame-phases.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class FramePhasesTests(unittest.TestCase):
    def test_solver_submetrics_and_details_children_are_not_added_twice(self):
        result = MODULE.analyze([
            "[gpui-frame] total=20.00ms prepaint=15.00ms paint=5.00ms scene_ops=4",
            "[gpui-phases] root_request_other=3.000ms layout_solve=5.000ms "
            "prepaint_other=6.000ms hit_test=1.000ms paint=4.500ms a11y_finish=0.500ms "
            "layout_measure=2.000ms layout_solve_ex_measure=3.000ms solve_calls=1 measure_calls=4",
            "[atlas-perf] frame_summary details.element.prepaint_ms=6.00 details.ring.sync_ms=2.00",
        ])
        self.assertEqual(result["draws_with_phase_split"], 1)
        self.assertEqual(result["worst_draws"][0]["phase_sum_ms"], 20)
        self.assertEqual(result["issues"], [])
        self.assertEqual(result["details_app_intervals_inclusive_spans"]["details.element.prepaint"]["max_ms"], 6)

    def test_old_capture_does_not_invent_solver_timings(self):
        result = MODULE.analyze(["[gpui-frame] total=24.54ms prepaint=15.78ms paint=8.76ms"])
        self.assertEqual(result["draws"], 1)
        self.assertEqual(result["draws_with_phase_split"], 0)
        self.assertIsNone(result["disjoint_draw_buckets"]["layout_solve"])

    def test_rejects_orphaned_and_incomplete_phases(self):
        result = MODULE.analyze([
            "[gpui-phases] paint=1.000ms",
            "[gpui-frame] total=1.00ms prepaint=0.00ms paint=1.00ms",
            "[gpui-phases] paint=1.000ms",
        ])
        self.assertEqual(len(result["issues"]), 2)
        self.assertEqual(result["draws_with_phase_split"], 0)

    def test_duplicate_phases_do_not_attach_twice(self):
        phase = "[gpui-phases] " + " ".join(
            f"{key}={1.0 if key == 'paint' else 0.0:.3f}ms" for key in MODULE.BUCKETS
        )
        result = MODULE.analyze([
            "[gpui-frame] total=1.00ms prepaint=0.00ms paint=1.00ms", phase, phase,
        ])
        self.assertEqual(result["draws_with_phase_split"], 1)
        self.assertEqual(len(result["issues"]), 1)

    def test_inconsistent_buckets_are_reported(self):
        result = MODULE.analyze([
            "[gpui-frame] total=20.00ms prepaint=10.00ms paint=10.00ms",
            "[gpui-phases] " + " ".join(f"{key}=0.000ms" for key in MODULE.BUCKETS),
        ])
        self.assertEqual(len(result["issues"]), 1)

    def test_empty_capture(self):
        self.assertIsNone(MODULE.analyze([])["draw_total"])


if __name__ == "__main__":
    unittest.main()
