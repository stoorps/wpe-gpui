#!/usr/bin/env python3
"""Run Cargo's actual executable, including tests and custom target paths."""
import sys
from local_runtime import launch

if len(sys.argv) < 2:
    sys.exit('Usage: cargo-runner.py EXECUTABLE [ARGUMENT ...]')
launch(sys.argv[1], sys.argv[2:])
