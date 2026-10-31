#!/usr/bin/env python3
"""Native Wayland integration test plus a remote-resource request recorder."""
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
import os
import subprocess
import sys
import threading

requests = []
class Recorder(BaseHTTPRequestHandler):
    def do_GET(self):
        requests.append(self.path)
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b'tracker')
    def log_message(self, *args):
        pass

root = Path(__file__).resolve().parent.parent
server = HTTPServer(('127.0.0.1', 18765), Recorder)
threading.Thread(target=server.serve_forever, daemon=True).start()
env = os.environ.copy()
env['G_DEBUG'] = 'fatal-criticals'
command = [str(root / 'target/debug/browser-demo'), '--self-test']
if '--local-runtime' in sys.argv:
    command = [sys.executable, str(root / 'scripts/run-local.py'), '--self-test']
try:
    result = subprocess.run(command, cwd=root, env=env, timeout=60)
finally:
    server.shutdown()
if requests:
    print(f'FAIL: email fixture made external requests: {requests}', file=sys.stderr)
    sys.exit(1)
print('PASS: email fixture made zero requests to the tracking endpoint', file=sys.stderr)
sys.exit(result.returncode)
