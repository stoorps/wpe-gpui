#!/usr/bin/env python3
"""Exercise native partial-construction cleanup in separate opt-in processes."""
import argparse
import json
import os
import signal
from pathlib import Path
import subprocess
import sys
import time

from process_groups import stop_group

ROOT = Path(__file__).resolve().parent.parent
STAGES = ('after-display', 'after-egl', 'before-webview', 'after-webview')


def run_stage(command, env):
    process = subprocess.Popen(command, cwd=ROOT, env=env, start_new_session=True)
    try:
        return process.wait(timeout=30)
    except subprocess.TimeoutExpired:
        return 124
    finally:
        stop_group(process)


def interrupted(signum, _frame):
    raise SystemExit(128 + signum)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'target/native-constructor-failures.json')
    args = parser.parse_args()
    signal.signal(signal.SIGTERM, interrupted)
    subprocess.run(['cargo', 'build', '-p', 'embedding-example', '--bin', 'embedding-native-test',
                    '--features', 'native-failure-injection'], cwd=ROOT, check=True)
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=ROOT))
    binary = Path(metadata['target_directory']) / 'debug/embedding-native-test'
    command = [sys.executable, str(ROOT / 'scripts/cargo-runner.py'), str(binary)]
    results = []
    for stage in STAGES:
        env = os.environ.copy()
        env.update(GPUI_WPE_FAIL_STAGE=stage, GPUI_WPE_FAIL_SKIP='2', G_DEBUG='fatal-criticals')
        started = time.monotonic()
        print(f'NATIVE_FAILURE_STAGE {stage}', flush=True)
        code = run_stage(command, env)
        results.append({'stage': stage, 'exit_code': code, 'passed': code == 0,
                        'duration_seconds': time.monotonic() - started,
                        'failed_constructors': 8, 'healthy_existing_views': 2})
    passed = all(item['passed'] for item in results)
    report = {'schema_version': 1, 'passed': passed, 'stages': results,
              'feature': 'native-failure-injection (disabled by default)',
              'scope': 'Repeated partial initialization failures with native live-object/source counts and distinct surviving pixel snapshots'}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + '\n')
    print(f'NATIVE_FAILURES passed={passed} report={args.output}', flush=True)
    return 0 if passed else 1


if __name__ == '__main__':
    sys.exit(main())
