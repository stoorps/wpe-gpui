#!/usr/bin/env python3
"""Run a command on a private headless Weston display, preserving its exit code."""
import argparse
import datetime
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import stat
import subprocess
import sys
import tempfile
import time

from process_groups import stop_group

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--weston-root', type=Path, help='Previously verified extracted Weston package; never downloaded by this script')
    parser.add_argument('--runtime-dir', type=Path, help='Private directory owned by you, mode0700; defaults to a temporary directory')
    parser.add_argument('--log', type=Path, default=ROOT / 'target/isolated-wayland/weston.log')
    parser.add_argument('--width', type=int, default=1600)
    parser.add_argument('--height', type=int, default=1000)
    parser.add_argument('--scale', type=int, default=1)
    parser.add_argument('--check', action='store_true', help='Start and inspect compositor, then exit without an application')
    parser.add_argument('--allow-capture', action='store_true', help='Authorize Weston debug/capture protocols in this private display only')
    parser.add_argument('command', nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ['--'] else args.command
    if not command and not args.check:
        parser.error('provide a command after --, or use --check')
    if command and args.check:
        parser.error('--check cannot be combined with a command')
    if min(args.width, args.height, args.scale) <= 0:
        parser.error('dimensions and scale must be positive')

    env = os.environ.copy()
    env.pop('DISPLAY', None)
    env.pop('WAYLAND_SOCKET', None)
    env['XDG_SESSION_TYPE'] = 'wayland'
    env['XDG_CURRENT_DESKTOP'] = 'Weston'
    server_env = env.copy()
    if args.weston_root:
        package = args.weston_root.expanduser().resolve()
        binary = package / 'usr/bin/weston'
        libraries = package / 'usr/lib'
        data = package / 'usr/share'
        if not binary.is_file() or not libraries.is_dir() or not data.is_dir():
            parser.error('--weston-root must contain usr/bin/weston, usr/lib and usr/share')
        if not shutil.which('bwrap'):
            parser.error('extracted Weston packages require bubblewrap, or install Weston normally')
        server = ['bwrap', '--bind', '/', '/', '--dev-bind', '/dev', '/dev',
                  '--overlay-src', '/usr/lib', '--overlay-src', str(libraries), '--ro-overlay', '/usr/lib',
                  '--overlay-src', '/usr/share', '--overlay-src', str(data), '--ro-overlay', '/usr/share',
                  '--', str(binary)]
        server_env['LD_LIBRARY_PATH'] = ':'.join([str(libraries), str(libraries / 'weston'), server_env.get('LD_LIBRARY_PATH', '')]).rstrip(':')
        source = str(package)
    else:
        installed = shutil.which('weston')
        if not installed:
            parser.error('Weston is not installed; install it or pass --weston-root with a verified extracted package')
        binary = Path(installed)
        server = [installed]
        source = 'installed'

    temporary = None
    if args.runtime_dir:
        runtime = args.runtime_dir.expanduser().absolute()
        runtime.mkdir(mode=0o700, parents=True, exist_ok=True)
        info = runtime.lstat()
        if not stat.S_ISDIR(info.st_mode) or info.st_uid != os.getuid() or stat.S_IMODE(info.st_mode) != 0o700:
            parser.error('--runtime-dir must be a non-symlink directory owned by you with mode0700')
    else:
        temporary = tempfile.TemporaryDirectory(prefix='wpe-weston-')
        runtime = Path(temporary.name)
    name = 'wayland-wpe-test'
    display = runtime / name
    session_file = runtime / '.gpui-wpe-isolated.json'
    if session_file.exists():
        parser.error('isolated session marker already exists; choose another runtime directory')
    if display.exists() or (runtime / (name + '.lock')).exists():
        parser.error(f'{display} or its lock already exists; choose another runtime directory')
    env['XDG_RUNTIME_DIR'] = server_env['XDG_RUNTIME_DIR'] = str(runtime)
    env['WAYLAND_DISPLAY'] = name
    server_env.pop('WAYLAND_DISPLAY', None)
    args.log = args.log.expanduser().absolute()
    args.log.parent.mkdir(parents=True, exist_ok=True)
    server += ['--backend=headless', '--renderer=gl', '--shell=desktop-shell.so',
               '--no-config', '--idle-time=0', f'--socket={name}', '--fake-seat',
               f'--width={args.width}', f'--height={args.height}', f'--scale={args.scale}']
    if args.allow_capture:
        server.append('--debug')
    compositor = client = None
    metadata = {'started_utc': datetime.datetime.now(datetime.timezone.utc).isoformat(),
                'compositor': 'Weston headless', 'weston_source': source,
                'server_command': server, 'command': command,
                'xdg_runtime_dir': str(runtime), 'wayland_display': name,
                'requested_renderer': 'gl', 'allow_capture': args.allow_capture, 'output': {'width': args.width, 'height': args.height, 'scale': args.scale},
                'log': str(args.log),
                'measurement_limitations': ['Virtual compositor, not physical display presentation or COSMIC behavior',
                                             'The GPU is still shared with the desktop and other workloads',
                                             'This is display isolation, not a security sandbox']}
    previous_handlers = {}
    def interrupted(signum, _frame):
        raise KeyboardInterrupt(signum)
    for signum in (signal.SIGINT, signal.SIGTERM):
        previous_handlers[signum] = signal.signal(signum, interrupted)
    result = 1
    try:
        with args.log.open('wb') as log:
            compositor = subprocess.Popen(server, env=server_env, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            deadline = time.monotonic() + 15
            while time.monotonic() < deadline:
                if compositor.poll() is not None:
                    raise RuntimeError(f'Weston exited before ready: {compositor.returncode}')
                if display.is_socket():
                    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as probe:
                        probe.settimeout(0.1)
                        try:
                            probe.connect(str(display))
                            break
                        except OSError:
                            pass
                time.sleep(0.02)
            else:
                raise RuntimeError('Weston did not create a listening Wayland socket within15seconds')
            time.sleep(0.2)
            if compositor.poll() is not None:
                raise RuntimeError(f'Weston failed during startup: {compositor.returncode}')
            lines = args.log.read_text(errors='replace').splitlines()
            renderer = [line for line in lines if any(key in line.lower() for key in ('gl renderer:', 'gl vendor:', 'egl device', 'egl version:', 'drm device', 'using rendering device'))]
            metadata['renderer_log'] = renderer
            metadata['server_pid'] = compositor.pid
            session_file.write_text(json.dumps({'runtime_dir': str(runtime), 'wayland_display': name,
                                                'server_pid': compositor.pid, 'weston_source': source,
                                                'allow_capture': args.allow_capture}) + '\n')
            env['GPUI_WPE_ISOLATED_SESSION'] = str(session_file)
            print(f'ISOLATED_WAYLAND runtime={runtime} display={name} log={args.log}', file=sys.stderr, flush=True)
            for line in renderer:
                print(f'ISOLATED_WAYLAND {line}', file=sys.stderr, flush=True)
            if args.check:
                result = 0
            else:
                client = subprocess.Popen(command, env=env, start_new_session=True)
                while client.poll() is None:
                    if compositor.poll() is not None:
                        raise RuntimeError('Weston exited while the application was running')
                    time.sleep(0.1)
                result = client.returncode
                # Let Weston consume the final Wayland disconnect/unmap events
                # before terminating its shell; no benchmark runs in this drain.
                time.sleep(0.2)
                if result < 0:
                    result = 128 - result
    except KeyboardInterrupt as error:
        result = 128 + (error.args[0] if error.args and isinstance(error.args[0], int) else signal.SIGINT)
    except (OSError, RuntimeError) as error:
        metadata['error'] = str(error)
        print(f'ISOLATED_WAYLAND error: {error}', file=sys.stderr)
        if args.log.exists():
            print('\n'.join(args.log.read_text(errors='replace').splitlines()[-35:]), file=sys.stderr)
    finally:
        stop_group(client)
        stop_group(compositor)
        metadata['compositor_exit_code'] = compositor.returncode if compositor else None
        metadata['exit_code'] = result
        metadata['finished_utc'] = datetime.datetime.now(datetime.timezone.utc).isoformat()
        args.log.with_suffix(args.log.suffix + '.json').write_text(json.dumps(metadata, indent=2) + '\n')
        for signum, handler in previous_handlers.items():
            signal.signal(signum, handler)
        session_file.unlink(missing_ok=True)
        if temporary:
            temporary.cleanup()
    return result


if __name__ == '__main__':
    sys.exit(main())
