import importlib.util
import json
from pathlib import Path
import unittest
from unittest import mock
import contextlib
import io
import tempfile

spec = importlib.util.spec_from_file_location('resize_benchmark', Path(__file__).resolve().parents[1] / 'scripts/benchmark-resize.py')
benchmark = importlib.util.module_from_spec(spec)
spec.loader.exec_module(benchmark)


def report():
    return {'schema_version': 1, 'endpoint': 'submitted', 'reference': {'refresh_hz': 60},
            'continuous': {'duration_ms': 30_000}, 'samples': [
                {'generation': i, 't0_us': i * 1_000_000, 't1_us': i * 1_000_000 + 1000,
                 'submitted_us': i * 1_000_000 + 50_000, 'correct': True,
                 'validation': 'fixture-marker', 'final': i == 199}
                for i in range(200)]}


class ResizeTraceTests(unittest.TestCase):
    def test_submission_is_labelled_and_budgets_pass(self):
        summary = benchmark.summarize(report())
        self.assertTrue(summary['resize_gate_passed'])
        self.assertEqual(summary['p95_ms'], 50)
        self.assertEqual(summary['endpoint_label'], 'resize-to-submitted-frame')
        self.assertTrue(summary['limitations'])

    def test_geometry_alone_is_not_layout_proof(self):
        trace = report()
        trace['samples'][0]['validation'] = 'buffer-size-only'
        summary = benchmark.summarize(trace)
        self.assertTrue(summary['performance_budgets_passed'])
        self.assertFalse(summary['resize_gate_passed'])

    def test_presented_endpoint_needs_feedback(self):
        trace = report()
        trace['endpoint'] = 'presented'
        summary = benchmark.summarize(trace)
        self.assertEqual(summary['missed_generations'], 200)
        self.assertFalse(summary['resize_gate_passed'])

    def test_out_of_order_stages_are_rejected(self):
        trace = report()
        trace['samples'][0]['t2_us'] = 500
        with self.assertRaises(ValueError):
            benchmark.summarize(trace)

    def test_superseded_and_missing_samples_remain_visible(self):
        trace = report()
        trace['samples'][0]['superseded'] = True
        del trace['samples'][1]['submitted_us']
        summary = benchmark.summarize(trace)
        self.assertEqual(summary['superseded_generations'], 1)
        self.assertEqual(summary['missed_generations'], 1)
        self.assertFalse(summary['resize_gate_passed'])

    def test_duplicate_generation_and_nonfinite_timing_rejected(self):
        trace = report()
        trace['samples'][1]['generation'] = 0
        with self.assertRaises(ValueError):
            benchmark.summarize(trace)
        trace = report()
        trace['samples'][1]['submitted_us'] = float('nan')
        with self.assertRaises(ValueError):
            benchmark.summarize(trace)

    def test_interrupted_driver_cannot_reuse_a_previous_trace(self):
        with tempfile.TemporaryDirectory() as directory:
            trace = Path(directory) / 'previous.json'
            trace.write_text(json.dumps(report()))
            with mock.patch.object(benchmark, 'run_driver') as driver, contextlib.redirect_stderr(io.StringIO()):
                code = benchmark.main(['--trace-output', str(trace)])
            driver.assert_called_once()
            self.assertEqual(code, 2)
            self.assertFalse(trace.exists())

    def test_omitted_final_settle_cannot_pass(self):
        trace = report()
        trace['samples'][-1]['final'] = False
        summary = benchmark.summarize(trace)
        self.assertEqual(summary['missing_final_fixtures'], ['default'])
        self.assertFalse(summary['resize_gate_passed'])

    def test_final_settle_and_continuous_duration_are_gates(self):
        trace = report()
        trace['samples'][-1]['submitted_us'] += 600_000
        trace['continuous']['duration_ms'] = 200
        summary = benchmark.summarize(trace)
        self.assertEqual(summary['final_failures'], [199])
        self.assertFalse(summary['performance_budgets_passed'])


if __name__ == '__main__':
    unittest.main()
