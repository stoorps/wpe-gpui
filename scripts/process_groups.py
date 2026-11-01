"""Bounded cleanup of process groups created with start_new_session=True."""
import os
import signal
import subprocess
import time


def stop_group(process, grace=3):
    if process is None:
        return
    group = process.pid
    try:
        # The leader may already have exited while Cargo/WebKit children live.
        os.killpg(group, signal.SIGTERM)
    except ProcessLookupError:
        pass
    deadline = time.monotonic() + grace
    while True:
        process.poll()  # Reap our child even while checking its descendants.
        try:
            os.killpg(group, 0)
        except ProcessLookupError:
            break
        if time.monotonic() >= deadline:
            try:
                os.killpg(group, signal.SIGKILL)
            except ProcessLookupError:
                pass
            break
        time.sleep(0.02)
    try:
        process.wait(timeout=1)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=1)
