"""Shared launch logic for Cargo and the explicit local-runtime launcher."""
import os
from pathlib import Path
import shutil
import sys

ROOT = Path(__file__).resolve().parent.parent


def launch(binary, arguments, require_local=False):
    binary = Path(binary).resolve()
    if not binary.is_file():
        sys.exit(f'Missing executable {binary}. Build first: cargo build -p browser-demo')
    package_lib = ROOT / '.deps/usr/lib'
    if not (package_lib / 'wpe-webkit-2.0/WPEWebProcess').is_file():
        if require_local:
            sys.exit('Missing .deps WPE runtime. Run scripts/setup-local-arch.sh first.')
        os.execv(str(binary), [str(binary), *arguments])
    if not shutil.which('bwrap'):
        sys.exit('The local WPE runtime requires bubblewrap (bwrap), or install system WPE and remove .deps/.')
    env = os.environ.copy()
    env['LD_LIBRARY_PATH'] = str(package_lib) + (':' + env['LD_LIBRARY_PATH'] if env.get('LD_LIBRARY_PATH') else '')
    # WPE's packaged helper paths are absolute. Overlay them privately; preserve
    # device access and leave WebKit's own process sandbox enabled.
    args = ['bwrap', '--bind', '/', '/', '--dev-bind', '/dev', '/dev',
            '--overlay-src', '/usr/lib', '--overlay-src', str(package_lib),
            '--ro-overlay', '/usr/lib', '--', str(binary), *arguments]
    os.execvpe('bwrap', args, env)
