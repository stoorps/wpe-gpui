# Commands run from the repository root. Recipe arguments remain shell arguments.
set positional-arguments

# Show available commands.
default:
    @just --list

# Run the browser demo; optionally pass a URI or --self-test.
run *args:
    cargo run -p browser-demo -- "$@"

# Open the local input interoperability lab.
lab:
    cargo run -p browser-demo -- demo://editing/

# Build the workspace.
build:
    cargo build --workspace

# Build an optimized browser demo.
build-release:
    cargo build --release -p browser-demo

# Format Rust sources.
fmt:
    cargo fmt --all

# Check Rust formatting without changing files.
fmt-check:
    cargo fmt --all --check

# Run strict workspace Clippy checks.
lint:
    cargo clippy --workspace --all-targets -- -D warnings

# Run unit and documentation tests.
test:
    cargo test --workspace

# Run formatting, lint, and unit checks (does not open a window).
check: fmt-check lint test test-tools

# Run the native interop and blocked-resource tests (opens a Wayland window).
test-interop *args: build
    python3 scripts/test-interop.py "$@"

# Load the default URI, check rendering, and exit after a short smoke run.
smoke:
    BROWSER_DEMO_SMOKE=1 G_DEBUG=fatal-criticals cargo run -p browser-demo

# Download, verify, and extract the local Arch WPE development runtime.
setup-local:
    ./scripts/setup-local-arch.sh

# Run deterministic updater and performance-report tests without a desktop.
test-tools:
    python3 -m unittest discover -s tests -p 'test_*.py'

# Record resize correctness/submission latency (opens a real Wayland window).
perf-resize:
    python3 scripts/benchmark-resize.py --trace-output target/resize-trace.json --output target/resize-summary.json

# Measure visible/hidden idle, input/scroll proxies and 1,000 lifecycle cycles.
perf-runtime *args:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p target
    cargo run -p browser-demo -- --benchmark-runtime --report target/runtime-trace.json "$@" 2>&1 | tee target/runtime.log

# Run 1,000 lifecycle cycles followed by a one-hour mixed-view soak.
perf-soak *args:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p target
    cargo run -p browser-demo -- --benchmark-runtime --soak-cycles 1000 --mixed-seconds 3600 --runtime-timeout-seconds 7200 --report target/soak-trace.json "$@" 2>&1 | tee target/soak.log

# Exercise isolated JS-disabled selected text, concurrent Copy and crash/reload.
test-native-features:
    G_DEBUG=fatal-criticals cargo run -p embedding-example --bin embedding-native-test

# Collect Rust dependency license texts and inventory for the Linux target.
licenses:
    python3 scripts/license-inventory.py --all-features --collect --output target/license-inventory.json --markdown target/license-inventory.md --notices-dir target/third-party-notices

# Close one OS window while another retains its WebView and managed handle.
test-window-lifecycle:
    G_DEBUG=fatal-criticals cargo run -p embedding-example --bin embedding-window-test

# Check partial native construction cleanup with opt-in fault injection.
test-native-failures:
    python3 scripts/test-native-failures.py --output target/native-failures.json

# Run a command on a private Weston display; pass options then -- COMMAND.
isolated *args:
    python3 scripts/run-isolated-wayland.py "$@"

# Run the same native interop/zero-network corpus against the optimized binary.
test-interop-release: build-release
    python3 scripts/test-interop.py --binary target/release/browser-demo

# Check provided-only filter readiness, cancellation and fail-closed compilation.
test-native-policy:
    python3 scripts/test-native-policy.py --output target/native-policy.json

# Deterministic local-page rendering smoke; the live smoke remains separate.
smoke-local:
    BROWSER_DEMO_SMOKE=1 G_DEBUG=fatal-criticals cargo run -p browser-demo -- demo://editing/
