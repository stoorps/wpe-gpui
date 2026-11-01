"""Regression proof for cleanup after a Cargo-like group leader already exited."""
import os
from pathlib import Path
import signal
import subprocess
import sys
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent.parent / 'scripts'))
from process_groups import stop_group


@unittest.skipUnless(sys.platform == 'linux', 'Linux process-group regression')
class ProcessGroupCleanup(unittest.TestCase):
    def test_orphaned_descendant_is_killed_after_leader_exit(self):
        child_code = 'import signal,time; signal.signal(signal.SIGTERM, signal.SIG_IGN); print("ready", flush=True); time.sleep(30)'
        leader_code = ('import subprocess,sys; '
                       f'child=subprocess.Popen([sys.executable,"-c",{child_code!r}],stdout=subprocess.PIPE,text=True); '
                       'assert child.stdout.readline().strip()=="ready"; '
                       'print(child.pid,flush=True)')
        process = subprocess.Popen([sys.executable, '-c', leader_code], stdout=subprocess.PIPE,
                                   text=True, start_new_session=True)
        try:
            child = int(process.stdout.readline().strip())
            self.assertEqual(process.wait(timeout=3), 0)
            self.assertEqual(os.getpgid(child), process.pid)
            started = time.monotonic()
            stop_group(process, grace=0.1)
            self.assertLess(time.monotonic() - started, 1)
            # An adopted zombie is dead and awaits its system reaper, rather
            # than retaining a running workload after the leader has exited.
            deadline = time.monotonic() + 1
            while time.monotonic() < deadline:
                try:
                    status = Path(f'/proc/{child}/status').read_text()
                except FileNotFoundError:
                    return
                if 'State:\tZ' in status:
                    return
                time.sleep(0.01)
            self.fail('descendant still running after bounded group cleanup')
        finally:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait(timeout=1)
            process.stdout.close()

    def test_already_gone_group_is_harmless(self):
        process = subprocess.Popen([sys.executable, '-c', 'pass'], start_new_session=True)
        process.wait(timeout=3)
        stop_group(process, grace=0.1)


if __name__ == '__main__':
    unittest.main()
