#!/usr/bin/env python3
"""Capture GPUI overlays from a runner-authorized PRIVATE Weston compositor."""
import argparse
import json
import os
from pathlib import Path
import selectors
import shutil
import signal
import stat
import subprocess
import sys
import time

from process_groups import stop_group

ROOT = Path(__file__).resolve().parent.parent
TRIGGERS = {
    'web selection/copy reaches host clipboard bridge': 'modal',
    'GPUI modal field receives typing': 'menu',
}


def private_session():
    marker = os.environ.get('GPUI_WPE_ISOLATED_SESSION')
    if not marker:
        raise RuntimeError('Use run-isolated-wayland.py --allow-capture; real desktop capture is refused')
    path = Path(marker)
    info = path.parent.lstat()
    if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
        raise RuntimeError('isolation runtime must be an owned 0700 directory')
    session = json.loads(path.read_text())
    if (not session['allow_capture'] or Path(session['runtime_dir']) != path.parent
            or os.environ.get('XDG_RUNTIME_DIR') != str(path.parent)
            or os.environ.get('WAYLAND_DISPLAY') != session['wayland_display']
            or os.environ.get('WAYLAND_SOCKET')):
        raise RuntimeError('private capture authorization/display does not match the isolated runner')
    os.kill(session['server_pid'], 0)
    return session


def capture_tool(session):
    env = os.environ.copy()
    env.pop('DISPLAY', None)
    env.pop('WAYLAND_SOCKET', None)
    if session['weston_source'] == 'installed':
        binary = shutil.which('weston-screenshooter')
        if not binary:
            raise RuntimeError('installed Weston has no weston-screenshooter')
        return [binary], env
    package = Path(session['weston_source'])
    binary = package / 'usr/bin/weston-screenshooter'
    libraries = package / 'usr/lib'
    if not binary.is_file():
        raise RuntimeError('verified Weston package has no screenshooter executable')
    env['LD_LIBRARY_PATH'] = ':'.join([str(libraries), str(libraries / 'weston'), env.get('LD_LIBRARY_PATH', '')]).rstrip(':')
    return [str(binary)], env


def capture(name, output, command, env):
    directory = output / name
    directory.mkdir(parents=True, exist_ok=False)
    # Self-test's checkpoint is emitted just before changing the GPUI overlay.
    # Its next stage waits600ms; allow the compositor several frames to latch it.
    time.sleep(0.2)
    process = subprocess.Popen([*command, '--source-type=framebuffer', '--buffer-type=shm'],
                               cwd=directory, env=env, stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, text=True, start_new_session=True)
    try:
        text, _ = process.communicate(timeout=5)
        (directory / 'capture.log').write_text(text)
        if process.returncode:
            raise RuntimeError(f'{name} capture failed: {text.strip()}')
    finally:
        stop_group(process)
    pictures = sorted(directory.glob('*.png'))
    if not pictures:
        raise RuntimeError(f'{name} capture produced no PNG in its private artifact directory')
    return [str(picture.relative_to(output)) for picture in pictures]


def interrupted(signum, _frame):
    raise SystemExit(128 + signum)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--output-dir', type=Path, default=ROOT / 'target/overlay-captures')
    args = parser.parse_args()
    try:
        session = private_session()
        shooter, capture_env = capture_tool(session)
    except (OSError, ValueError, KeyError, RuntimeError) as error:
        parser.error(str(error))
    output = args.output_dir.expanduser().absolute()
    output.mkdir(parents=True, exist_ok=True)
    # Refuse stale stages rather than misreporting an earlier capture as new proof.
    if any((output / name).exists() for name in TRIGGERS.values()):
        parser.error('capture stage directory already exists; select a fresh --output-dir')
    signal.signal(signal.SIGTERM, interrupted)
    command = [sys.executable, str(ROOT / 'scripts/test-interop.py')]
    process = subprocess.Popen(command, cwd=ROOT, env=os.environ.copy(), stdout=subprocess.PIPE,
                               stderr=subprocess.STDOUT, bufsize=0, start_new_session=True)
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    pictures, failures = {}, []
    pending = b''
    code = 1
    deadline = time.monotonic() + 75
    try:
        with (output / 'interop.log').open('w') as log:
            while time.monotonic() < deadline:
                ready = selector.select(timeout=0.1)
                if not ready:
                    if process.poll() is not None:
                        break
                    continue
                chunk = os.read(process.stdout.fileno(), 65536)
                if not chunk:
                    break
                pending += chunk
                while b'\n' in pending:
                    raw, pending = pending.split(b'\n', 1)
                    line = raw.decode('utf-8', errors='replace') + '\n'
                    log.write(line)
                    log.flush()
                    print(line, end='', flush=True)
                    for trigger, name in TRIGGERS.items():
                        if trigger in line and name not in pictures:
                            try:
                                pictures[name] = capture(name, output, shooter, capture_env)
                                print(f'OVERLAY_CAPTURE {name}: {pictures[name]}', flush=True)
                            except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
                                failures.append(str(error))
            else:
                failures.append('interop capture timed out')
        try:
            code = process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            code = 124
            failures.append('interop did not exit before deadline')
    finally:
        stop_group(process)
        selector.close()
        process.stdout.close()
    for name in TRIGGERS.values():
        if name not in pictures:
            failures.append(f'missing {name} compositor capture')
    report = {'schema_version': 1, 'interop_exit_code': code, 'captures': pictures,
              'failures': failures, 'source': 'actual Weston output framebuffer including GPUI and WPE surfaces',
              'runtime_dir': session['runtime_dir'], 'wayland_display': session['wayland_display'],
              'visual_verification': 'Requires human/model inspection of captured PNGs; file creation alone is not visual proof',
              'limitations': ['Private headless Weston only', 'Capture intentionally perturbs rendering; exclude from performance timing']}
    (output / 'capture-report.json').write_text(json.dumps(report, indent=2) + '\n')
    return 0 if code == 0 and not failures else 1


if __name__ == '__main__':
    sys.exit(main())
