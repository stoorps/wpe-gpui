#!/usr/bin/env python3
"""Exercise native filter readiness, queued loads, close, and async failure."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from process_groups import stop_group

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output', type=Path, default=ROOT / 'target/native-policy.json')
    args = parser.parse_args()
    subprocess.run(['cargo', 'build', '-p', 'embedding-example', '--bin', 'embedding-native-test',
                    '--features', 'native-failure-injection'], cwd=ROOT, check=True)
    metadata = json.loads(subprocess.check_output(['cargo', 'metadata', '--no-deps', '--format-version', '1'], cwd=ROOT))
    binary = Path(metadata['target_directory']) / 'debug/embedding-native-test'
    results = []
    for mode in ('queued', 'failure'):
        with tempfile.TemporaryDirectory(prefix='wpe-policy-test-') as directory:
            env = os.environ.copy()
            env.update(GPUI_WPE_POLICY_TEST=mode, G_DEBUG='fatal-criticals', TMPDIR=directory)
            if mode == 'failure':
                env['GPUI_WPE_POLICY_FAULT'] = 'compile'
            else:
                env.pop('GPUI_WPE_POLICY_FAULT', None)
            command = [sys.executable, str(ROOT / 'scripts/cargo-runner.py'), str(binary)]
            process = subprocess.Popen(command, cwd=ROOT, env=env, start_new_session=True)
            try:
                code = process.wait(timeout=20)
            except subprocess.TimeoutExpired:
                code = 124
            finally:
                stop_group(process)
            remaining = [path.name for path in Path(directory).glob('gpui-wpe-policy-*')]
            results.append({'mode': mode, 'exit_code': code, 'remaining_policy_stores': remaining,
                            'passed': code == 0 and not remaining})
    passed = all(item['passed'] for item in results)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps({'schema_version': 1, 'passed': passed, 'cases': results}, indent=2) + '\n')
    print(f'NATIVE_POLICY passed={passed} report={args.output}', flush=True)
    return 0 if passed else 1


if __name__ == '__main__':
    sys.exit(main())
