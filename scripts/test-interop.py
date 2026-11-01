#!/usr/bin/env python3
"""Native Wayland interop plus HTTP requests and speculative TCP recording."""
import argparse
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
import os
import signal
import subprocess
import sys
import threading
from process_groups import stop_group

ROOT = Path(__file__).resolve().parent.parent


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/browser-demo')
    parser.add_argument('--local-runtime', action='store_true')
    args = parser.parse_args()
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error(f'Missing executable {binary}; build it first')
    requests, connections = [], []

    class Recorder(BaseHTTPRequestHandler):
        timeout = 2
        def record(self):
            requests.append((self.command, self.path))
            self.send_response(200)
            self.end_headers()
            if self.command != 'HEAD':
                self.wfile.write(b'tracker')
        do_GET = do_POST = do_HEAD = do_OPTIONS = record
        def log_message(self, *args):
            pass

    class Server(HTTPServer):
        def get_request(self):
            connection = super().get_request()
            connections.append(connection[1])
            return connection

    server = Server(('127.0.0.1', 18765), Recorder)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    env = os.environ.copy()
    env['G_DEBUG'] = 'fatal-criticals'
    command = [sys.executable, str(ROOT / 'scripts/cargo-runner.py'), str(binary), '--self-test']
    if args.local_runtime:
        command = [sys.executable, str(ROOT / 'scripts/run-local.py'), '--binary', str(binary), '--self-test']
    child, result = None, 1
    previous = {}
    def interrupted(signum, _frame):
        raise KeyboardInterrupt(signum)
    for signum in (signal.SIGINT, signal.SIGTERM):
        previous[signum] = signal.signal(signum, interrupted)
    try:
        child = subprocess.Popen(command, cwd=ROOT, env=env, start_new_session=True)
        result = child.wait(timeout=60)
        if result < 0:
            result = 128 - result
    except subprocess.TimeoutExpired:
        print('FAIL: native interop timed out', file=sys.stderr)
        result = 124
    except KeyboardInterrupt as error:
        result = 128 + (error.args[0] if error.args else signal.SIGINT)
    finally:
        stop_group(child)
        server.shutdown()
        server.server_close()
        for signum, handler in previous.items():
            signal.signal(signum, handler)
    if requests or connections:
        print(f'FAIL: email fixture network activity: HTTP={requests}, TCP connections={len(connections)}', file=sys.stderr)
        return 1
    print('PASS: email fixture made zero HTTP requests and zero TCP connections to the tracking endpoint', file=sys.stderr)
    return result


if __name__ == '__main__':
    sys.exit(main())
