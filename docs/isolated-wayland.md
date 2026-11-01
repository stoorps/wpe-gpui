# Isolated Wayland diagnostics

Use the private headless Weston runner when testing resize, idle behavior, or a long soak. It creates its own Wayland socket and a 0700 runtime directory, so desktop pointer movement, focus changes, output scaling, and window management do not alter the test window. The real GPU remains shared with other applications.

The runner uses an installed `weston` by default. It also accepts an extracted Weston package which you have already verified through your distribution's package signatures. It never downloads packages or installs files into the system. The extracted-package path requires `bwrap`; its library and data directories are overlaid only in the compositor process's mount namespace.

```sh
# Installed Weston: inspect compositor startup and renderer, then stop.
python3 scripts/run-isolated-wayland.py --check

# Previously verified extracted package; this path is an example.
python3 scripts/run-isolated-wayland.py \
  --weston-root /path/to/verified-weston-package --check

# Dedicated JavaScript-disabled native API checks.
G_DEBUG=fatal-criticals python3 scripts/run-isolated-wayland.py \
  --weston-root /path/to/verified-weston-package \
  --log target/isolated-wayland/features-weston.log -- \
  cargo run -p embedding-example --bin embedding-native-test

# Resize correctness: the normal Cargo runner still selects the WPE runtime.
G_DEBUG=fatal-criticals python3 scripts/run-isolated-wayland.py \
  --weston-root /path/to/verified-weston-package \
  --log target/isolated-wayland/resize-weston.log -- \
  cargo run -p browser-demo -- --benchmark-resize \
    --report target/isolated-wayland/resize-trace.json

# Long mixed workload; keep the normal benchmark's own timeout.
G_DEBUG=fatal-criticals python3 scripts/run-isolated-wayland.py \
  --weston-root /path/to/verified-weston-package \
  --log target/isolated-wayland/soak-weston.log -- \
  cargo run -p browser-demo -- --benchmark-runtime \
    --soak-cycles 1000 --latency-samples 200 --idle-seconds 10 \
    --mixed-seconds 3600 --runtime-timeout-seconds 7200 \
    --report target/isolated-wayland/soak-trace.json
```

Defaults are a 1600×1000 output at scale 1, headless GL rendering, a fake input seat, desktop shell, no user configuration, and no idle blanking. Desktop shell allows ordinary window resize; kiosk shell would force fullscreen. `--width`, `--height`, and integer `--scale` can select another controlled output. Each invocation starts and stops its own compositor, propagates the application exit status, and terminates its process groups on interruption. An existing private directory can be selected with `--runtime-dir`; the runner refuses an existing socket or lock.

The Weston log and adjacent `.log.json` preserve the invocation, actual renderer log, runtime/display identity, dimensions, exit code, and limitations. Inspect the renderer line: selecting `--renderer=gl` alone does not prove hardware acceleration. The compositor renderer and the client renderer must be assessed separately. Recorded hardware identities belong in the report for each run.

A passing isolated run establishes behavior on headless Weston. It does not establish physical display presentation latency, COSMIC behavior, fractional scaling, or the wider compositor/hardware matrix. This wrapper isolates the display environment; it is not a security sandbox. WebKit retains its normal process sandbox and the existing WPE runtime launcher.

For native investigation, `GPUI_WPE_TRACE=1` records incoming WebKit frame damage and host geometry, clip, focus, input, and evaluation metadata. It excludes page text, script source, and URI contents. Trace logging can perturb timing and should be disabled for measured runs.

Idle checks must wait for delayed engine animations to finish. WPE WebKit 2.54.2's overlay scrollbar begins hiding after a two-second delay and then animates for three seconds. A 750ms quiet interval can therefore end before the animation starts. The runtime benchmark requires a longer quiet interval rather than disabling normal scrollbar behavior. See the [exact upstream implementation](https://github.com/WebKit/WebKit/blob/wpewebkit-2.54.2/Source/WebCore/platform/generic/ScrollbarsControllerGeneric.cpp#L162).

## Partial-construction failures

The optional `native-failure-injection` Cargo feature is disabled by default. Its C hooks are absent from ordinary builds. The test launcher starts separate processes with a fixed host-selected failure stage; it does not modify Rust's process environment while native threads are running.

```sh
G_DEBUG=fatal-criticals python3 scripts/run-isolated-wayland.py \
  --weston-root /path/to/verified-weston-package \
  --log target/isolated-wayland/constructor-failures-weston.log -- \
  python3 scripts/test-native-failures.py \
    --output target/isolated-wayland/constructor-failures.json
```

Each process first renders two distinct healthy views, then rejects eight constructors at one of four stages: display ownership, EGL plus a small snapshot allocation, before WebView construction, and after full WebView initialization. It verifies the diagnostic error, exact bridge object/source/payload counts, and both surviving pixel snapshots. This exercises 32 partial-construction failures through the usual destructor; it does not simulate arbitrary WebKit/driver internal failures or GPU device loss.

## Compositor overlay captures

Capture uses the complete Weston output, including both GPUI and WPE surfaces. It requires explicit authorization on a new private display; the capture script refuses an absent or mismatched isolated-session marker. Build `browser-demo` first, then run:

```sh
G_DEBUG=fatal-criticals python3 scripts/run-isolated-wayland.py \
  --weston-root /path/to/verified-weston-package --allow-capture \
  --log target/overlay-proof/weston.log -- \
  python3 scripts/capture-interop.py --output-dir target/overlay-proof/captures
```

The script runs the ordinary interop test and tracking recorder, captures its GPUI modal and menu checkpoints, and preserves the interop exit status, PNGs, capture logs, and a JSON report. Use a fresh output directory for each run. Capture file creation is not itself visual proof: inspect the PNGs to verify that overlays cover web content correctly. The runner's capture flag enables privileged Weston debug protocols only inside that private test display. Keep capture disabled for performance measurements; readback intentionally changes rendering behavior.

## Interpreting results

Require the child exit status and final report, and inspect capture images.
Partial progress logs do not establish a completed soak. Keep recorded results
with their renderer/runtime metadata and source identity; see the
[evidence index](evidence/README.md) for the historical reference runs.
Headless results do not establish physical presentation, fractional output
transitions, desktop clipboard ownership, or broad hardware compatibility.
