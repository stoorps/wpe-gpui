# Testing and verification

Run commands from the repository root. Native checks require the dependencies in
[getting started](getting-started.md). Keep tests for code correctness separate
from real desktop input, graphics, and security claims.

## Routine checks

```sh
just check
cargo check --workspace --all-targets --locked
cargo check -p embedding-example --features upstream-compat --locked
cargo doc -p gpui-wpe --no-deps
```

`just check` runs Rust formatting, strict workspace/all-target Clippy, Rust unit/doc
tests, and Python tool tests. It opens no windows, but Rust compilation still needs
the native development dependencies. The optional upstream proof needs additional
GTK/WebKitGTK development packages; see [compatibility](compatibility.md).

## Native checks

| Command | Exercises |
| --- | --- |
| `just smoke-local` | Deterministic local rendering, completed load and clean exit. |
| `just smoke` | Live FullSphere diagnostic; remote site behavior can cause failure. |
| `just test-interop` | GPUI-dispatched input, overlays, navigation/history, resources and raw restricted fixtures; HTTP requests and TCP accepts at the controlled endpoint. |
| `just test-interop-release` | The interop corpus against an optimized demo binary. |
| `just test-native-features` | Independent views, scripting-disabled selection/find/edit state, deferred Copy, process crash/reload and history identity. |
| `just test-window-lifecycle` | OS-window removal while a surviving window retains an entity/handle; exactly-once close and survivor health. |
| `just test-native-failures` | Opt-in partial-construction fault injection and survivor/object-count checks. |
| `just test-native-policy` | Filter readiness, close while pending, document replacement, shared compilation, fail-closed compiler errors and store cleanup. |
| `just perf-resize` / `just perf-runtime` | Measurement workloads and regression gates; see [performance](performance.md). |

Require a successful process exit as well as assertions and a fresh final report.
A pass printed before a teardown crash is a failed run. A title or submitted frame
does not prove successful load completion. Keep failed and interrupted results with
their diagnostics; do not overwrite them with unrelated later passes.

Avoid interacting with native test windows during measurements. Desktop focus,
occlusion and output scaling can change observed behavior. Use a private compositor
for repeatability:

```sh
just isolated --check
just isolated --log target/isolated-wayland/interop-weston.log -- just test-interop
just isolated --log target/isolated-wayland/policy-weston.log -- just test-native-policy
```

The [isolated Wayland guide](isolated-wayland.md) covers verified extracted Weston
packages, output scale, constructor failures, and compositor captures. Private
headless output is not physical desktop or presentation evidence.

Fault injection is behind `native-failure-injection`, disabled in ordinary builds.
Use the dedicated scripts rather than changing process environment after native
threads start. Normal builds ignore the diagnostic constructor/policy fault flags.

## Manual desktop checks

- Click actual web content, select and copy text into another application. Copy
  new text there and paste into a web input. Synthetic dispatch cannot establish
  a compositor-issued clipboard ownership serial.
- Open a tall GPUI menu and modal above content; inspect text, shadows, translucent
  backdrop and clipping. Type/paste in the modal, close with Escape, and confirm
  focus and page editing resume without input leakage.
- Resize continuously, maximize/restore, hide/show, and move between intended
  output scales. Test pointer capture and selection release outside the viewport.
- Review Unicode/RTL/CJK/emoji reading, selection, copy, find and fallback fonts.
  IME/accessibility exclusions must stay visible to an adopting app.
- Test the app's external-link/compose flow, remote-image consent and revocation,
  message replacement, resource-limit UI, renderer failure and host text fallback.

Record OS/compositor, physical/scale configuration, native runtime, GPU/driver,
source/build identity, test actions and result. User-reported manual clipboard
success is distinct from automation and from another environment's support.

## CI and artifacts

The [CI workflow](../.github/workflows/ci.yml) defines an Arch system-WPE container
lane for checks and upstream type compatibility. Its separate manual native job
requires a self-hosted Linux x86_64 runner labelled `wayland`, an exported live
session, rendering devices/drivers, fonts, native dependencies, `just`, and Weston.
The native performance commands start private compositor displays.

The optional `soak` workflow input requests the hour workload; it is not part of an
ordinary check. Select it deliberately when a longer measurement is needed and
retain the final report and exit status separately from any earlier partial run.

A workflow definition does not demonstrate a hosted run. Configure permissions
before relying on automation, and do not expose a desktop self-hosted runner to
untrusted PR code. Bot PR validation also has token restrictions described in
[runtime updates](runtime-updates.md).

Retain raw logs, trace/progress/final JSON, renderer/runtime metadata, captures,
exit status and source/build hashes. Reports under ignored `target/` can disappear
on clean; export release evidence before removing them. The committed
[summaries](evidence/README.md) preserve dated checkpoints, including failures and
limitations, rather than certifying the current release tree.
