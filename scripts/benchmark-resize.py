#!/usr/bin/env python3
"""Summarize resize traces without confusing submission with presentation.

Without --input, run browser-demo's benchmark driver and read its JSON report.
This program checks trace consistency and budgets; it cannot manufacture visual
layout evidence or compositor presentation timestamps that the driver did not collect.
"""
import argparse
from collections import Counter
import json
import math
import os
import signal
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
STAGES = ('t0_us', 't1_us', 't2_us', 'layout_us', 'received_us',
          'acquire_ready_us', 'submitted_us', 'presentation_us')
VALIDATIONS = {'fixture-marker', 'visual-capture', 'buffer-size-only'}


def number(value, name):
    if isinstance(value, bool) or not isinstance(value, (int, float)) or not math.isfinite(value) or value < 0:
        raise ValueError(f'{name} must be a finite nonnegative number')
    return value


def percentile(values, fraction):
    if not values:
        return None
    return sorted(values)[max(0, math.ceil(fraction * len(values)) - 1)]


def summarize(report):
    if not isinstance(report, dict) or report.get('schema_version') != 1:
        raise ValueError('Expected resize trace schema_version 1')
    endpoint = report.get('endpoint')
    if endpoint not in {'submitted', 'presented'}:
        raise ValueError('endpoint must be submitted or presented')
    samples = report.get('samples')
    if not isinstance(samples, list) or not samples:
        raise ValueError('samples must be a nonempty array')
    reference = report.get('reference', {})
    if not isinstance(reference, dict):
        raise ValueError('reference must be an object')
    if 'refresh_hz' in reference:
        number(reference['refresh_hz'], 'reference.refresh_hz')
    continuous = report.get('continuous', {})
    if not isinstance(continuous, dict):
        raise ValueError('continuous must be an object')
    duration = number(continuous.get('duration_ms', 0), 'continuous.duration_ms')
    latencies = []
    superseded = 0
    incorrect = 0
    missed = 0
    proxy = 0
    seen = set()
    stage_times = {name: [] for name in STAGES[1:]}
    fixtures = Counter()
    final_failures = []
    final_samples = Counter()
    expected_fixtures = set()
    end_key = 'presentation_us' if endpoint == 'presented' else 'submitted_us'
    for i, sample in enumerate(samples):
        if not isinstance(sample, dict):
            raise ValueError(f'sample {i} must be an object')
        generation = sample.get('generation')
        if isinstance(generation, bool) or not isinstance(generation, int) or generation < 0:
            raise ValueError(f'sample {i} needs a nonnegative integer generation')
        fixture = str(sample.get('fixture', 'default'))
        expected_fixtures.add(fixture)
        identity = (str(sample.get('document_id', '0')), generation)
        if identity in seen:
            raise ValueError(f'duplicate document/generation {identity}')
        seen.add(identity)
        start = number(sample.get('t0_us'), f'sample {i}.t0_us')
        previous = start
        for stage in STAGES[1:]:
            value = sample.get(stage)
            if value is None:
                continue
            value = number(value, f'sample {i}.{stage}')
            if value < previous:
                raise ValueError(f'sample {i} has out-of-order timestamp {stage}')
            previous = value
        if any(type(sample.get(flag, False)) is not bool for flag in ('superseded', 'correct', 'final')):
            raise ValueError(f'sample {i} needs boolean correct/superseded/final flags')
        if sample.get('superseded', False):
            superseded += 1
            if sample.get('final', False):
                final_failures.append(generation)
            continue
        if not sample.get('correct', False):
            incorrect += 1
        validation = sample.get('validation', 'buffer-size-only')
        if validation not in VALIDATIONS:
            raise ValueError(f'sample {i} has an unknown layout-validation method')
        if validation == 'buffer-size-only':
            proxy += 1
        end = sample.get(end_key)
        if end is None:
            missed += 1
            if sample.get('final', False):
                final_failures.append(generation)
            continue
        elapsed_ms = (number(end, f'sample {i}.{end_key}') - start) / 1000
        latencies.append(elapsed_ms)
        fixtures[fixture] += 1
        if sample.get('final', False):
            final_samples[fixture] += 1
        if sample.get('final', False) and (not sample.get('correct', False) or elapsed_ms > 500):
            final_failures.append(generation)
        for stage in stage_times:
            if sample.get(stage) is not None:
                stage_times[stage].append((sample[stage] - start) / 1000)
    stats = {
        'p50_ms': percentile(latencies, .50), 'p95_ms': percentile(latencies, .95),
        'p99_ms': percentile(latencies, .99), 'max_ms': max(latencies) if latencies else None,
    }
    sufficient_samples = bool(fixtures) and all(fixtures[fixture] >= 200 for fixture in expected_fixtures)
    missing_final = sorted(expected_fixtures - final_samples.keys())
    budgets_passed = (bool(latencies) and stats['p95_ms'] <= 100 and stats['p99_ms'] <= 200
                      and stats['max_ms'] <= 500 and not missed and not incorrect
                      and not final_failures and not missing_final and sufficient_samples and duration >= 30_000)
    full_gate = budgets_passed and proxy == 0
    return {
        'schema_version': 1,
        'endpoint': endpoint,
        'endpoint_label': 'resize-to-presented-frame' if endpoint == 'presented' else 'resize-to-submitted-frame',
        'reference': reference, 'total_generations': len(samples),
        'measured_samples': len(latencies), 'samples_per_fixture': dict(fixtures),
        'superseded_generations': superseded, 'missed_generations': missed,
        'incorrect_generations': incorrect, 'geometry_only_generations': proxy,
        'continuous_duration_ms': duration, 'final_failures': final_failures,
        'final_samples_per_fixture': dict(final_samples), 'missing_final_fixtures': missing_final,
        **stats,
        'stage_p95_ms_from_geometry': {name: percentile(values, .95) for name, values in stage_times.items()},
        'performance_budgets_passed': budgets_passed,
        'resize_gate_passed': full_gate,
        'limitations': (["Submission is a proxy; no actual compositor presentation timing is established."]
                       if endpoint == 'submitted' else [])
                      + (["Buffer size alone does not prove correct WebKit layout."] if proxy else []),
    }


def run_driver(trace):
    command = ['cargo', 'run', '-p', 'browser-demo', '--', '--benchmark-resize', '--report', str(trace)]
    # Kill the entire Cargo/browser process group on timeout, preserving any trace.
    process = subprocess.Popen(command, cwd=ROOT, start_new_session=True)
    try:
        code = process.wait(timeout=300)
    except BaseException:
        try:
            os.killpg(process.pid, signal.SIGTERM)
            process.wait(timeout=5)
        except ProcessLookupError:
            pass
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait()
        raise
    if code:
        raise subprocess.CalledProcessError(code, command)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input', type=Path, help='Existing driver JSON trace; otherwise launch the native driver')
    parser.add_argument('--output', type=Path, help='Write the summary JSON here')
    parser.add_argument('--trace-output', type=Path, help='Preserve the newly recorded raw trace here')
    parser.add_argument('--allow-proxy', action='store_true', help='Allow geometry-only budget checks to exit zero; never mark the full gate passed')
    args = parser.parse_args(argv)
    try:
        with tempfile.TemporaryDirectory(prefix='gpui-wpe-resize-') as directory:
            trace = args.input
            if trace is None:
                trace = args.trace_output.resolve() if args.trace_output else Path(directory) / 'trace.json'
                trace.parent.mkdir(parents=True, exist_ok=True)
                trace.unlink(missing_ok=True)
                run_driver(trace)
            summary = summarize(json.loads(trace.read_text()))
        output = json.dumps(summary, indent=2, sort_keys=True) + '\n'
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(output)
        print(output, end='')
        return 0 if summary['resize_gate_passed'] or (args.allow_proxy and summary['performance_budgets_passed']) else 1
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f'resize benchmark: {error}', file=sys.stderr)
        return 2


if __name__ == '__main__':
    sys.exit(main())
