# Recorded verification evidence

These summaries capture local working-tree checks on 2026-10-31 and preserve raw
source-artifact hashes. They are not release artifacts tied to an immutable commit.
The [validation record](../history/2026-10-31-validation.md) explains the checkpoint;
the current [support matrix](../support.md) and [roadmap](../roadmap.md) describe
capabilities and unfinished work.

| Summary | What it records |
| --- | --- |
| [Runtime](2026-10-31-runtime-summary.json) | Completed 1,000-cycle, idle, input and scroll workload; not an hour soak. |
| [Resize, scale 1](2026-10-31-resize-summary.json) | Active-window Weston GL layout-marker/submission proxy. |
| [Resize, scale 2](2026-10-31-resize-scale2-summary.json) | Separate integer-scale configuration; not fractional scaling/output movement. |
| [Failed desktop resize](2026-10-31-resize-desktop-diagnostic.json) | Interrupted COSMIC diagnostic retained alongside isolated passes. |
| [Native integration](2026-10-31-integration-summary.json) | Native feature/window lifecycle checks and one live consented image normalization. |
| [Constructor failures](2026-10-31-constructor-failures.json) | Four fault stages and 32 partial-construction failures with healthy survivors. |
| [Policy failures/readiness](2026-10-31-native-policy.json) | Pending close/replacement, shared filter job and fail-closed compiler errors; HTTP/TCP results are interpreted in the validation record. |
| [Overlays and manual clipboard](2026-10-31-overlay-manual-summary.json) | Inspected compositor captures and user-reported two-way physical desktop clipboard exchange. |
| [Interrupted mixed workload](interrupted-soak.json) | 26 mixed minutes after a 1,000-cycle prefix, without a final report or exit status. Further hour testing was waived for that delivery. |

Raw traces, screenshots and logs remain outside Git, usually under ignored
`target/` or local temporary paths. Export them separately before cleaning build
artifacts or preparing a release. The JSON's original artifact paths identify the
historical measurement sources; moving these summaries does not regenerate them.

DOM observation and frame submission are not actual compositor presentation.
Snapshot counters cannot prove absence of transient leaks. A failed or interrupted
run stays failed/partial, and a later pass does not erase its unresolved behavior.
See [measurement methodology](../performance.md) and [testing](../testing.md).
