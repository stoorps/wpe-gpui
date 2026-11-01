#!/usr/bin/env python3
"""Explicit launcher using the extracted Arch WPE runtime."""
import argparse
from pathlib import Path
from local_runtime import ROOT, launch

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--binary', type=Path, default=ROOT / 'target/debug/browser-demo')
args, remaining = parser.parse_known_args()
launch(args.binary, remaining, require_local=True)
