"""The overlay helper must reject the real desktop before opening clients."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parent.parent


class CaptureGuard(unittest.TestCase):
    def run_guard(self, directory, session=None):
        output = directory / 'must-not-be-created'
        env = os.environ.copy()
        env.pop('GPUI_WPE_ISOLATED_SESSION', None)
        if session is not None:
            marker = directory / 'session.json'
            marker.write_text(json.dumps(session))
            env['GPUI_WPE_ISOLATED_SESSION'] = str(marker)
        result = subprocess.run([sys.executable, str(ROOT / 'scripts/capture-interop.py'),
                                 '--output-dir', str(output)], env=env,
                                capture_output=True, text=True, timeout=3)
        self.assertEqual(result.returncode, 2)
        self.assertFalse(output.exists())
        return result

    def test_unmarked_desktop_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_guard(Path(directory))
            self.assertIn('real desktop capture is refused', result.stderr)

    def test_unapproved_private_session_is_refused(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_guard(Path(directory), {'allow_capture': False})
            self.assertIn('authorization/display does not match', result.stderr)


if __name__ == '__main__':
    unittest.main()
