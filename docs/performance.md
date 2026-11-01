# Performance measurements

`scripts/benchmark-resize.py` summarizes an existing JSON trace with `--input`, or
launches `browser-demo --benchmark-resize --report TRACE` when no input is given.
The native driver owns actual measurement. The summary tool never treats an EGL
swap or a differently sized buffer as proof of correct compositor presentation.

```sh
python3 scripts/benchmark-resize.py --input trace.json --output summary.json
# Run on a real desktop and retain both measurement and summary artifacts.
python3 scripts/benchmark-resize.py --trace-output trace.json --output summary.json
# Explicit diagnostic-only mode: allow geometry-proxy budgets, never mark full gate passed.
python3 scripts/benchmark-resize.py --input trace.json --allow-proxy
```

Schema version 1 has `endpoint` (`submitted` or `presented`), `reference` metadata,
`samples`, and `continuous.duration_ms`. Each sample needs a nonnegative document-
local `generation`, `t0_us` from accepted viewport geometry, `correct`, and a
`validation` value: `fixture-marker`, `visual-capture`, or `buffer-size-only`.
Use `document_id` and `fixture` when several documents/fixtures are measured.

Optional monotonic timestamps in order are `t1_us` (native geometry), `t2_us` (WPE
resize delivered), `layout_us`, `received_us`, `acquire_ready_us`, `submitted_us`,
and `presentation_us`. Missing stages stay missing. A `presented` endpoint requires
real `presentation_us` values correlated to the actual submitted frame and clock
domain; otherwise choose `submitted`, whose label remains a submission proxy.

Superseded generations are recorded with `superseded: true`, counted separately,
and excluded from latency percentiles. Non-superseded missing/incorrect samples
remain failures rather than disappearing from the report. Mark final-settle
samples with `final: true`; every measured fixture needs at least one final sample. Record renderer errors, trace/capture artifacts,
actual reference hardware/refresh/runtime versions, and measurement limitations
alongside the trace. A fixture marker or capture must establish new document
layout rather than only buffer dimensions; the parser trusts this evidence field
but cannot generate or independently verify the visual evidence.

The configured resize gate requires at least 200 measured samples per fixture, at least
30 seconds of continuous resize, p95 <= 100 ms, p99 <= 200 ms, max <= 500 ms, and
no missed/incorrect/final-settle failures. It reports p50/p95/p99/max, sample counts,
superseded generations and cumulative stage p95 values. These are provisional local
regression budgets, not an industry standard or a hardware-wide performance promise.

`resize_gate_passed` is false for buffer-size-only traces. `--allow-proxy` can make
the CLI exit successfully for their numeric budget check, but the JSON full-gate
field remains false and explicitly warns that layout is unproven. Submitted-frame
reports likewise retain a warning that actual compositor presentation is unknown;
the release evidence must include separate visual layout checks where permitted
by the declared measurement endpoint. Do not relabel submission as presentation.

The manual native CI lane runs the driver and retains raw trace and summary artifacts.
Deterministic parser/gate tests cover timing order, nonfinite values, duplicate generations,
missing feedback, proxy labeling, sample/superseded accounting and final budgets.

## Runtime and lifecycle diagnostic

`just perf-runtime` runs the trusted local runtime fixture and saves
`target/runtime-trace.json`. It exercises 200 native GPUI-dispatched typing samples,
200 wheel samples, ten seconds each of visible and hidden idle, and 1,000
create/load/close cycles. Every fifth cycle waits for a correctly loaded first
frame; the other cycles close during loading to exercise cancellation. The view
uses a bounded viewport over a 20,000-pixel document. `just perf-soak` adds a
one-hour mixed-view run alternating tabs, creation/close and GPUI modal overlays.
It has a two-hour overall safety timeout. The ordinary runtime suite has a
30-minute timeout; per-operation setup/frame deadlines keep failures bounded.

A shorter development run is explicit and records the reduced workload:

```sh
just perf-runtime --soak-cycles 20 --latency-samples 10 --idle-seconds 1 --runtime-timeout-seconds 120
```

The fixture first blurs editables and waits for three seconds without a new submitted
frame or viewport generation change (with a 15-second warm-up deadline). The diagnostic's timer is completely
suspended during each idle interval. It
captures process CPU ticks, aggregate thread context switches, native submission
counts and process/descendant FD/RSS snapshots at the interval boundaries. Context
switches are a wakeup **proxy**, and CPU covers the app process rather than WebKit
children. Thread termination can decrease the aggregate context-switch counter;
such a trace does not pass the proxy gate. Descendant RSS double-counts shared
mappings and includes allocator/driver caches; snapshots are not a native object
allocation census. Closed GPUI entities are checked through weak references after
a three-second settle interval, and FD/RSS checkpoints are recorded every 25 cycles.
Exact bridge-owned view/display/import/buffer/source/resource/document counters
are captured beside those checkpoints. View/document/resource counts must return
to the warmed baseline; pending frames, explicit bridge GLib sources and resource
waits cannot grow, imports stay bounded per live view, and at most one shared
display is permitted. Source counts exclude WebKit/driver internal sources.

Typing and wheel events enter through GPUI's real dispatcher. Trusted diagnostic
JavaScript reads the resulting value/scroll position; wheel samples also require
a new DOM wheel event. Their endpoint is **dispatch-to-DOM-observed upper bound**,
including JS IPC and up to ten milliseconds of observation polling. These are
neither input-to-display measurements nor evidence that a particular compositor
frame was presented. Native scroll submission timestamps produce separate interval
percentiles and record missed intermediate submissions; they may include unrelated
frames. CPU/GPU/kernel/runtime metadata and all these limitations accompany JSON.
Available DRM PCI IDs identify available devices, not the active EGL renderer.

Provisional local regression budgets are CPU <=5% of one core, aggregate context
switch proxy <=50/s and <=2 submissions per idle interval; DOM observation p95
<=100ms, p99 <=200ms and maximum <=500ms; observed scroll submission intervals p95
<=50ms, p99 <=100ms and maximum <=200ms. Native pending buffers must stay <=4 and
resident imports <=8. After warm-up/settling, app FD growth must stay <=16, app RSS
growth <=64MiB, descendant FD growth <=64 and summed descendant RSS growth <=256MiB.
Caches need explanation when they exceed a budget; limits are not automatically
raised. `proxy_gates.passed` evaluates the configured run. Separate workload flags
show whether 1,000 cycles/200 samples/ten-second idle and the one-hour mixed soak
were actually requested and completed. A passing short run cannot establish the
full workload, compositor presentation, hardware matrix or a no-leak guarantee.

Headless Rust unit tests cover Linux CPU-stat parsing (including spaces and
parentheses in process names), percentile calculation and configuration bounds.
The manual desktop CI lane retains runtime and optional one-hour soak JSON beside
the resize artifacts. A workflow definition is not evidence that these runs passed.

## Native event diagnostics

Set `GPUI_WPE_TRACE=1` before launching to write native geometry/clip/focus,
load/action/input metadata and incoming WPE damage rectangles to stderr. Text and
script contents and URIs are not logged by this trace; text/evaluation records
include byte counts. It includes pointer coordinates and native pointer identities,
is opt-in, and is diagnostic output rather than a stable public event format.
Tracing can affect timing, so retain it separately from the ordinary benchmark.

The idle investigation found delayed scrollbar-only damage after a quiet period.
The current benchmark waits for three seconds with neither submitted frames nor
viewport generation changes before measuring idle, covering the observed delayed
fade rather than starting inside its initial pause. Do not treat a shorter quiet
interval as evidence of steady idle. A diagnostic can use `GPUI_WPE_TRACE=1 just
perf-runtime ...`; the normal library has no mandatory trace polling loop.

Runtime runs also append `REPORT.progress.jsonl` alongside their report: lifecycle
checkpoints every 25 cycles, mixed-workload start/end and every 60 mixed steps,
plus baseline bridge-owned live counters. Stdout gives concise 60-second progress
heartbeats. Interruptions fail the run rather than accepting a stale report left
by a previous run. Progress records help distinguish a live long soak from a hang;
they are not a replacement for the successful final report and exit status.

## Isolated compositor run

`scripts/run-isolated-wayland.py` starts a private Weston headless compositor with
GL rendering and a fake seat, launches the command on its display and preserves
the command's exit status. It records compositor/renderer metadata beside its log.
With Weston installed, for example:

```sh
python3 scripts/run-isolated-wayland.py --log target/isolated-wayland/resize-weston.log -- \
  python3 scripts/benchmark-resize.py \
  --trace-output target/isolated-wayland/resize-trace.json \
  --output target/isolated-wayland/resize-summary.json
```

A previously verified extracted package can be supplied with `--weston-root`; the
script does not download or establish trust in that package. Display isolation
keeps desktop focus/scale changes out of the test, but shares the GPU with other
workloads. It is not a security sandbox, a physical display test or a substitute
for the declared desktop compositor/scale support matrix. Record actual renderer
logs rather than inferring the active GPU from available DRM device metadata.

## Recorded results

See the [dated validation checkpoint](history/2026-10-31-validation.md) and
[committed evidence summaries](evidence/README.md). The interrupted 26-minute
mixed workload is partial evidence, without a final report or clean-exit result.
The hour soak remains optional; no further hour run was requested for that checkpoint.
