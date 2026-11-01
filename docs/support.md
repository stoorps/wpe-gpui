# Support and limitations

The repository is an experimental 0.1.0 candidate. Implemented mechanisms are
usable for evaluation, but API stability, broad hardware coverage, and a completed
production release are not established. Query per-view `capabilities()` where an
operation is optional. Evidence here refers to the
[2026-10-31 checkpoint](history/2026-10-31-validation.md).

## Platform scope

| Configuration | Scope |
| --- | --- |
| Linux x86_64, native Wayland | Current implementation target. |
| WPE WebKit/WPEPlatform 2.54+ | Required API baseline; recorded runtime 2.54.2. Every newer runtime is not automatically validated. |
| GPUI Kit 0.8.1, Fast/Platform 0.1.12 | Current dependency identity; see [compatibility](compatibility.md). |
| Arch, COSMIC, AMD/Mesa | Local desktop reference; two-way physical clipboard was user-reported. Desktop resize diagnostics also contain an unresolved failure. |
| Headless Weston 15.0.1, Radeon 840M | Recorded GL resize at integer scales 1 and 2, native lifecycle/policy checks, and visually inspected compositor overlays. |
| Fractional/mixed-DPI, output movement, other GPUs/compositors | Not comprehensively verified. Integer scale-2 testing does not establish output-transition correctness. |
| X11/XWayland, Windows, macOS | No WPE backend support. An app can use upstream gpui-webview on other platforms through its own adapter. |
| Software rendering / multi-GPU | No guaranteed fallback or established compatibility matrix. |

## Capability matrix

“Implemented” describes mechanisms present in source and bounded checks. It does
not imply every input, malformed page, driver fault, or application workload has
been validated.

| Capability | Current behavior / limit |
| --- | --- |
| Managed entity and lifecycle | Automatic app/window hooks, weak handles, deterministic close and typed closed state. 1,000-cycle prefix and dedicated window/constructor checks recorded. |
| Web content and navigation | URL/HTML/document loading, native history, load state/events, explicit host navigation decisions. |
| Supplied resources | Generated document/base/resource schemes, byte quotas and bounded async responses. Arbitrary base URI, streaming/range and HTTP-response APIs are absent. |
| Restricted rendering | Disabled scripting plus `ProvidedOnly`; native filter readiness and fail-closed loading. Fixture HTTP/TCP recorder evidence, not a complete security audit. |
| Multiple views | Shared engine infrastructure with per-view ephemeral sessions. No persistent profile/cookie/data-management API. |
| GPUI overlays | Inspected menu, modal, shadow, and backdrop captures; host owns focus and modal interaction. |
| Geometry / clipping / scale | Implemented and measured on isolated integer-scale configurations; desktop resize propagation and physical mixed-scale behavior need further review. |
| Input / editing | Pointer/wheel, keyboard, focus, text editing, plain-text Copy/Cut/Paste, undo/redo and Select All. Full web-app key semantics and every layout are unverified. |
| Clipboard exchange | User reported real Copy/Paste both ways on COSMIC. Rich HTML/images, primary selection, and drag/drop are unavailable. |
| Selected-text query | Async event, only in an engine session that has never enabled page scripting. One query at a time; protocol timeout/cancellation can invalidate the shared clipboard until restart. |
| Find / hit information | Native find, next/previous, match count, basic context/hit data. Broad multilingual reading/copy/search coverage remains incomplete. |
| Theme / appearance | Background and app-owned CSS supported. Native per-view Light/Dark and isolated media-query preference return `Unsupported`; Auto follows shared WPE settings. |
| IME | Basic text/preedit forwarding exists. Surrounding-text integration, exact caret geometry, candidate placement and full composition/layout correctness are not complete. |
| Accessibility | No native web accessibility tree bridge. An app requiring screen-reader access needs a separate accessible host text path. |
| Failure recovery | Typed load/process/renderer/clipboard events. Reload or configured recreation depends on failure kind; catastrophic GPU hangs have no promised graceful recovery. |
| Native prompts / privileges | JS dialogs, file choosers, auth, permissions and downloads cancelled/denied; fullscreen disabled. Rich host hooks absent. |
| Production script/IPC APIs | No isolated worlds, initialization timing, bounded page/native messaging, or concurrent typed evaluations. Trusted diagnostic evaluation exists for scripting-enabled views. |
| Other browser features | No supported printing/PDF export, inspector UI, intrinsic content-height API, full media/capture controls or advanced proxy/certificate/session controls. |

## Email application scope

The separate `email-renderer` supports bounded HTML/inline-CSS sanitization,
CID/data raster images, static PNG normalization, remote-image placeholders and
document-scoped consent, and plain-text fallback. It does not parse MIME, send mail,
provide a composer, validate arbitrary attachment actions, or guarantee newsletter
pixel fidelity. The app owns those responsibilities and external-link UX.

Remote styles/fonts, SVG, animation, active embeds, CSS background images and
`srcset` are outside the reference reader flow. See [email rendering](email-rendering.md)
for limits and [security](security.md) for the core boundary. Neither the example
nor the generic crate constitutes a security-reviewed email product.

## Interpreting verification

Recorded tests include a completed short lifecycle/runtime suite, isolated resize
submission proxies, native policy/failure checks, inspected overlays, and manual
clipboard feedback. The 26-minute mixed workload was interrupted and has no final
report/clean-exit result. The hour soak was waived for that delivery and remains
unvalidated. A later source edit requires relevant new checks.

Compositor presentation timestamps were not collected. A submitted correct frame
is not proof of physical presentation latency. Hosted CI and bot-created PRs need
repository configuration and actual execution. See [testing](testing.md),
[performance](performance.md), and the [release roadmap](roadmap.md).
