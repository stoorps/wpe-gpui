#!/usr/bin/env python3
"""Explicit launcher for the debug binary using the extracted Arch WPE runtime."""
import sys
from local_runtime import ROOT, launch

launch(ROOT / 'target/debug/browser-demo', sys.argv[1:], require_local=True)
