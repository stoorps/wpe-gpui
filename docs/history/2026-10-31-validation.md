# Validation checkpoint: 2026-10-31

Historical working-tree assessment recorded on 2026-10-31. See the current
[support limits](../support.md) and [release roadmap](../roadmap.md) when evaluating
the project. This record separates implementation, verification and remaining gates
at the checkpoint; it is not a live support matrix. The current tree is an experimental candidate; these checkpoints
do not establish a completed release or a security-reviewed email product.

## Implemented mechanisms

| Area | Present in this candidate | Qualification |
| --- | --- | --- |
| Workload-neutral core | Builder/settings, typed errors/events, navigation decisions, document identity, bounded async resource delivery and library-owned lifecycle/event-loop integration. | Email sanitization, CID rules and remote-content consent belong to the separate app-level `email-renderer`. Public API stability remains experimental. |
| Upstream alignment | GPUI Kit 0.8.1 with GPUI Fast/Fast Platform 0.1.12; independent embedding app, exact GPUI type checks and optional actual `gpui-webview`/`lb-wry` construction proof. | Linux compilation is checked. WPE does not accept raw Wry builders/handles/errors or promise feature parity. macOS/Windows execution is unverified. |
| Lifecycle | Automatic quit/window integration, closed-handle detection, document cancellation and composition cleanup before host removal. | The 1,000-cycle and retained-entity/window tests pass at their checkpoints. 32 injected partial-construction failures pass. The hour soak was interrupted after 26 mixed minutes; further hour testing is waived for this delivery at the user’s request, without a completion claim. |
| Rendering/event loop | Device/surfaceless EGL display, borrowed host Wayland presenter, async acquire/completion fences, bounded queues/import cache and GLib readiness/deadline dispatch on the UI thread. | No ordinary host-supplied 16 ms pump or unconditional normal-path `glFinish()`. Tested proxy performance is below; broad GPU support and graceful catastrophic GPU-loss recovery are not established. |
| Generic policy | `ProvidedOnly` requires disabled scripting; per-document schemes/IDs, quotas, resource responders and stale-generation rejection. | Native restricted fixtures refuse network navigation/evaluation and record zero tracking-endpoint requests. This is not a complete hostile-content/origin isolation proof. |
| Reader/input APIs | Selection/copy/paste bridge, native find, focus/visibility, hit/context information, editing availability and typed load/process/renderer failures. | Native and synthetic integration checks pass. The user reports physical cross-application Copy/Paste working both ways on COSMIC. Broader input/visual coverage and timeout/cancellation races remain gates. Full IME/accessibility are later scope. |
| Selection-query capability | Page-script-free query via local clipboard interception in a dedicated scripting-disabled engine session. | WPE uses one shared primary display/clipboard. Any scripting-enabled view permanently disables query capability until engine restart; timeout/cancellation also disable it against late uncorrelated writes. Mixed browser sessions retain ordinary Copy/Paste. |
| Appearance | Auto follows shared native settings. Application-owned email CSS provides light/dark reading surfaces. | Per-view native Light/Dark explicitly return `Unsupported`: real media-query testing showed WPE reads primary-display settings globally. Separate display pools cannot provide isolation. |
| Email preparation | HTML/inline-CSS sanitization, bounded CID/data raster resources and a 128 MiB aggregate decoded-image cap, static PNG normalization including JPEG EXIF orientation, escaped plain text and host-rendered fallback. | App tests cover policy/resource/image limits. SVG, active embeds and animation are excluded from this flow. Real newsletter fidelity and hostile corpus review remain open. |
| Image consent | Revocable document-scoped consent, app-owned bounded HTTP(S) worker, validated/pinned public addresses, redirect/TLS checks, no environment proxy/cookies/referrers and validated image decoding. | One live FullSphere JPEG passes fetch-to-PNG normalization. This does not establish every DNS/redirect/slow-network or cancellation case. WebKit remains restricted. |
| Demo integration | GPUI navigation/URL/tabs, menus/submenus/popovers/modals, find, context menu, mailto compose intent, blocked-image controls, host plain-text fallback and retry/recreate-view failure actions. | Synthetic interop and focused native tests exist; physical input, drawing and mixed-scale behavior still need review. Compose is a demonstration UI, not mail transport. |
| Update automation | Dependabot and scheduled/manual native-runtime patch PR workflow; locked runtime pin, checksum/signature-verified local installation and loaded-runtime provenance. | Local tooling tests pass. Hosted runs, permissions and actual PR creation require repository setup after pushing. The contract is reviewable proposals, not automatic merges or a single-maintainer response SLA. |
| Licensing/consumption | Preserved Apache/Zed/Longbridge notices, direct/transitive dependency notes, packaged core/bridge README/license and reproducible target-filtered Cargo inventory/notice collection. | All-feature Linux inventory finds 713 packages and flags 43 missing package-root notice bundles for review. Native components and exact shipped-artifact obligations need separate materials; no complete BOM/legal-compliance claim. |

## Recorded verification

These are implementation checkpoints, not a claim that every check ran after every
subsequent edit. The final release tree must pass one coherent validation run.

- Recorded final-check checkpoint: **52 Rust tests and 31 Python tests passed**,
  with release build successful. Core unit tests number 10; email tests number 36,
  including JPEG EXIF orientation and aggregate decoded-image bounds. A release
  native-feature run also passed all 16 checks. Later policy fixes require their
  own final validation; earlier passes do not cover subsequent edits.
- `cargo check -p embedding-example --features upstream-compat --locked` checks
  the actual upstream constructor signature against identical GPUI types. It does
  not test native macOS/Windows behavior.
- The independent native embedding smoke renders local HTML without demo controls,
  manual pumping or manual teardown.
- Expanded native interop passed with zero failures: ordinary typing/copy/modal/
  resize/tab/history checks, raw restricted policy, JS-disabled find, invalid-load
  transaction, oversized geometry rejection, out-of-order resources, stale resource
  cancellation and closed-handle/late-key behavior. The recorder observed zero
  tracking requests; this is bounded fixture evidence, not universal isolation.
- [Native integration summary](../evidence/2026-10-31-integration-summary.json):
  **16 native feature checks** passed under `G_DEBUG=fatal-criticals`, including
  two-view content, UTF-8 query/editor state, no host-copy event during queries,
  deferred Copy, typed crash/reload recovery, retained restrictions, and document
  identity/history restoration after process restart. **Nine OS-window lifecycle
  checks** passed, including automatic native release when an entity is retained
  by a surviving window and exactly-once `Closed` delivery. Snapshot pixels verify
  fixture content; they are not resize or compositor presentation proof.
- A live app-owned consent example fetched FullSphere's JPEG logo and normalized
  it to **64,843 bytes of PNG**. This is one successful public image, not a general
  remote-fetch security certification.
- Both core/bridge `cargo package --list --allow-dirty --offline` checks include
  README and preserved Apache license. `just licenses` is repeatable, records
  sources/versions/features/lock hash and copies discovered notices with SHA256.

- [Partial-construction failures](../evidence/2026-10-31-constructor-failures.json):
  32 injected failures across four stages passed while two existing views stayed
  healthy, with native live-object/source checks. This does not cover every GPU,
  fence or teardown failure.
- [Overlay and manual verification](../evidence/2026-10-31-overlay-manual-summary.json):
  the root agent inspected actual compositor PNGs and confirmed GPUI modal,
  backdrop/shadow and long menu over WPE. The **user reported** physical desktop
  COSMIC `just lab` Copy/Paste working from web content to another application and
  back. This closes the basic cross-application clipboard gate for that setup;
  it is manual user evidence, not synthetic compositor-serial proof.
- A blocked-script-dialog test exposed a native crash; declining the dialog
  synchronously without closing it inline fixed the reproduced case, followed by
  a passing interop run. This is a checked regression, not comprehensive fuzzing.

A TCP recorder subsequently found a speculative preconnect privacy escape beyond
what an HTTP request recorder could see. A native content-filter enforcement fix
has been implemented and the reproduced fixture records zero HTTP requests and
TCP connections in both debug and optimized runs. The
[native policy suite](../evidence/2026-10-31-native-policy.json) passes 19 assertions
covering pending close/replacement, one shared compilation job, async compiler
failure, blank/fail-closed behavior, released waiters and private-store cleanup.
DNS packets were not separately captured; the exact engine's DNS hint path was
verified to consult the native content filter. Do not use the earlier HTTP-only result as complete network-isolation
proof or treat the pre-fix soak binary as coverage for this change.

An earlier unattended external clipboard sentinel was unreliable; the later manual
user report is distinct evidence. No hosted bot/workflow run is claimed. A live
FullSphere smoke displayed the Newsletter title and 191 frames without a reported
error over 12 seconds, but still had `loading=true`; this is not a load-completion
pass. A challenge iframe/external script is a possible explanation, not a proven
root cause or evidence about restricted-core enforcement.

## Runtime and lifecycle measurements

The [runtime summary](../evidence/2026-10-31-runtime-summary.json) records a completed
**242.38-second COSMIC Wayland run** with WPE 2.54.2 on a Ryzen AI 5 340 system.
It satisfies the configured short-suite workload and all its provisional proxy
gates; this short-suite report does not establish a one-hour soak.

| Measurement | Recorded result |
| --- | --- |
| Create/load/close cycles | 1,000 completed: 200 rendered, 800 closed during loading |
| Closed GPUI entities retained after settling | 0 |
| Typing / wheel observations | 200 / 200 matched |
| Input p95 / p99 | 11.083 / 15.333 ms |
| Scroll p95 / p99 | 31.092 / 32.439 ms |
| Visible / hidden idle | 10 seconds each; 0 submitted frames in each interval |
| App idle CPU | Approximately 0.20% / 0.10% of one core |
| App context-switch proxy | Approximately 3.30 / 4.60 per second |
| Settled app FD / RSS change | −2 FDs / +2.96 MiB |
| Descendant-inclusive FD / summed RSS change | 0 FDs / +3.51 MiB |
| Bridge-owned live counters | Return to warmed baseline |
| Peak outstanding buffers / cached imports | 2 / 4 |

Input/scroll endpoints are **dispatch-to-DOM-observed upper bounds**, including
GPUI dispatch, WebKit handling, diagnostic JS IPC and up to 10 ms observation
polling. They exclude compositor presentation. CPU covers the app process, not
WebKit subprocesses. Context switches are a wakeup proxy; descendant RSS counts
shared mappings more than once. FD/RSS budgets are regression checks, not proof
of no leaks. The CPU/device metadata identifies available hardware, not the active
EGL renderer of every component. Hidden means the webview's visibility is disabled;
it does not mean that the desktop OS window was minimized.

## Interrupted mixed workload: what 26 minutes showed

The [interrupted soak summary](../evidence/interrupted-soak.json) preserves hashes for
raw JSONL progress, log, dirty-source/build provenance and interruption metadata.
After a completed 1,000-cycle prefix, it recorded **26 mixed-workload minute
checkpoints** (1,560 seconds). Across those checkpoints, bridge-owned counts stayed
at one view, one display and five imports, with zero pending frames, owned sources
or resource waits. App FDs stayed at **83–84**. App RSS decreased from approximately
**221.3 to 169.8 MiB**, with a **168.8–221.6 MiB** range.

These snapshots show no accumulating bridge-owned objects or app FD/RSS growth in
that interval. They are useful partial durability evidence, not proof of no
transient leaks. The run ended without a final report or exit status, so it is
**not a successful full soak**. The user explicitly requested no further hour
soak: its completion requirement is waived for this delivery and remains
unvalidated. No additional hour run should be inferred from the shorter checks.
Later policy fixes are outside the interrupted binary's recorded build provenance.

## Resize measurements

The [isolated resize summary](../evidence/2026-10-31-resize-summary.json) records an
active-window run in **Weston 15 headless with GL rendering**, output 1600×1000 at
scale 1. Weston logs identify Mesa/AMD Radeon 840M on `/dev/dri/renderD128`.
It is a real GPU-backed virtual compositor, with no physical presentation feedback.

| Measurement | Recorded result |
| --- | --- |
| Endpoint / fixture | Resize-to-submitted-frame proxy / one original responsive email fixture |
| Samples / continuous duration | 779 / 30,013 ms |
| Active-window samples | 779 of 779 |
| p50 / p95 / p99 | 3.406 / 10.239 / 16.843 ms |
| Maximum | 26.482 ms |
| Incorrect / missed / superseded generations | 0 / 0 / 0 |
| Final-settle failures | 0 |
| Correct-layout check | Responsive raster marker plus dimensions |
| Presentation feedback / actual refresh rate | Not collected / not queried |

This run passes the configured submission-proxy resize gate. Layout completion and
actual presentation timestamps remain unavailable. Diagnostic readback can perturb
later samples. One warm fixture and one scale do not establish a newsletter,
cold-load, mixed-DPI or physical compositor support matrix.

A separate [integer-scale-2 run](../evidence/2026-10-31-resize-scale2-summary.json)
also passes the submission-proxy gate: **725 measured samples** over 30,034 ms,
p95 **18.742 ms**, p99 **30.228 ms**, maximum **105.048 ms**, zero incorrect/missed
and 15 superseded generations. This adds one scale configuration; it does not
verify fractional scaling or movement between outputs.

Retain the [failed desktop diagnostic](../evidence/2026-10-31-resize-desktop-diagnostic.json)
alongside the pass: its 762 generations include 673 inactive-window observations,
531 superseded generations and 60 missed/incorrect generations, so it **fails**.
During the interrupted COSMIC run, requested window bounds changed while GPUI
viewport/native geometry propagation stopped, with no observed frame backlog.
Focus/scale changes and background geometry throttling need further investigation;
the trace does not establish a root cause. An even earlier diagnostic had a doubled
resize-sequence bug and is excluded from performance claims. The isolated pass does
not erase the failed desktop behavior or establish physical COSMIC resize coverage.

## Remaining 0.1.0 gates

1. **Final coherent tree:** format, strict workspace Clippy, Rust/Python tests,
   default/upstream embedding checks, native interop/feature/window tests, successful
   and failing smoke exit codes, and updated performance artifacts after final edits.
   Passing output followed by teardown failure is a failed run.
2. **Durability and failures:** broader teardown/fence/resource-race and acquire/
   renderer failure behavior. The 1,000-cycle suite and 32 constructor failures pass
   their scopes. The user waived further hour testing for this delivery after the
   interrupted 26-minute mixed workload; hour completion remains unvalidated, not
   a pending authorized rerun or proof of graceful catastrophic GPU-loss recovery.
3. **Reader/visual/input coverage:** extend the user-reported two-way physical
   clipboard result and inspected compositor overlays with query timeout/cancel,
   Unicode/RTL/CJK/emoji reading/copy/search, keyboard traversal,
   pointer capture/zoom, popup/modal drawing, continuous physical resize and supported
   fractional/output scale transitions. Preserve explicit capability limits.
4. **Performance coverage:** presentation-aware evidence where available, broader
   fixture/scale/cold-load measurements and UI responsiveness under acquire delays.
   Reproduce and resolve or narrowly bound the interrupted desktop resize behavior;
   never relabel the DOM/submission proxies as displayed latency.
5. **App security/fidelity:** hostile markup/CSS/resource/network corpus, origin and
   replacement/cancellation review, remote-consent DNS/redirect/slow-network testing,
   newsletter fidelity and host-fallback review. A WebKit-rendered fallback cannot
   protect against an unsupported/vulnerable engine; the demo also has a GPUI fallback.
6. **Consumption and external setup:** independent clean supported install/update,
   exact shipped artifact license/notice/source materials, loaded runtime provenance,
   and repository setup for bot PR permissions/validation. No hosted execution or
   actual bot-created PR is established. A self-hosted Wayland lane must be configured
   before it can be called operational.

Full accessibility, full IME, persistent profiles, rich IPC/script worlds,
printing/export, rich clipboard/drag-drop, software fallback and broader hardware
coverage remain explicitly outside the narrow experimental scope. Consumers must
account for these limitations in their own supported users and deployments.

## Reproducing evidence

Use `just` to list commands. `just check`, `just test-interop`,
`just test-native-features`, `just test-window-lifecycle`, `just perf-runtime`,
`just perf-resize`, `just perf-soak` and `just licenses` cover the relevant routes.
See [performance methodology](../performance.md) and the justfile for
isolated-compositor options and limitations. Committed JSON summaries retain source
artifact hashes; raw traces/logs remain in ignored `target/` or local temporary
paths and must be retained separately for a release. These are working-tree
checkpoints, not artifacts cryptographically tied to a release commit.
