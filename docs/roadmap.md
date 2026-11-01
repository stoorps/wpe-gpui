# Feature matrix and release roadmap

The first release target is experimental **0.1.0**: a reusable workload-neutral
embedding crate with safe lifetime, reliable bounded rendering/input, minimal
generic policy enforcement, and a usable API. Broader browser embedding,
accessibility, hardware coverage, and packaging follow in later 0.x/v1 milestones.

The matrix distinguishes code present today from a completed release contract.
“Required” remains a release concern even when implementation exists; its declared
scope needs relevant evidence. The [support guide](support.md) documents current
limits, and [dated evidence](evidence/README.md) records what was actually checked.

## Core 0.1.0 matrix

| Capability | Include in 0.1.0 | Current implementation and remaining work |
| --- | --- | --- |
| Workload-neutral crate boundary | Required | Core excludes email rules, sanitizer and default URI. Keep this boundary as new hooks land. |
| Managed lifecycle and multiple views/windows | Required | Hooks, weak handles, close/cancellation and constructor cleanup implemented. Expand teardown/resource-race and fence-failure evidence; retain exact ownership order. |
| Builder, entity, state/events and typed errors | Required | Implemented with an independent consumer and upstream construction/type proof. Stabilize documentation and verify final source/consumer builds. |
| URL/HTML loading, native history and phases | Required | Implemented. Preserve transaction/state/identity correctness through reload, failure and Back/Forward. |
| Document identity and scoped resources | Required | Generated custom base URL and bounded resources implemented. Arbitrary caller-selected base URI is absent; keep that explicit rather than promising the original proposed API. |
| Async resource provider and cancellation | Required | Bounded exactly-once responders, expiry, replacement and close handling implemented. Extend late-response and failure-race review. |
| Immutable scripting and network contract | Required | Disabled JS plus ProvidedOnly implemented; native content filter readiness/failure tested. Expand hostile network/resource corpus without weakening enforcement. |
| Navigation, external-link and new-window intent | Required | Handler/delegation implemented; app owns launch UX. Preserve restricted-view denial and document/context validation. |
| Ephemeral sessions and message isolation | Required | Per-view ephemeral sessions, generated document identities and revocation implemented. Review reset/history/cache semantics; no process-per-view claim. |
| Resize, scale, clipping and bounded long pages | Required | Geometry and isolated integer-scale checks implemented. Resolve or narrowly bound failed desktop geometry propagation; verify declared physical/output behavior. |
| GPUI overlays and input occlusion | Required | Menus/modal/backdrop captures inspected and synthetic routing checked. Extend pointer/focus/keyboard/manual coverage. |
| Basic keyboard, focus and editing | Required | Implemented for lab flows. Expand traversal, capture, shortcuts, layout and focus-restoration checks while declaring full IME exclusions. |
| Selection, Copy/Cut/Paste and query | Required, limited | Real two-way clipboard reported; async query has engine-session restrictions. Verify timeout/cancel behavior and expose capability limits. |
| Find, hit/context information and zoom | Required | Implemented. Broaden Unicode/RTL/CJK/emoji and pointer/zoom correctness evidence. |
| Appearance and readable text | Required, basic | Background and app CSS supported. Per-view native Light/Dark explicitly unsupported; verify fonts and light/dark reader surfaces. |
| Process/load/render/clipboard failures | Required | Typed failure and demo recovery paths implemented. Recheck policy after recovery; do not advertise graceful catastrophic GPU-loss recovery. |
| Readiness-driven native servicing | Required | Library-owned readiness/deadline dispatch replaces normal 16 ms polling. Preserve bounded UI work and measure idle behavior. |
| Async GPU completion and import caching | Required | Fences, bounded pending queue/cache implemented. Broaden driver/acquire/backpressure coverage; exceptional safety fallback stays documented. |
| Resize/input/scroll/idle performance | Required | Workloads and regression budgets implemented. Re-run relevant final-tree measurements and preserve endpoint limits; perf is not deferred. |
| Privileged prompts and unsupported hooks | Required deny/cancel | JS dialogs, file chooser, auth, permissions, downloads cancelled; fullscreen disabled. No silent grant or native prompt fallback. |
| Native update automation and provenance | Required | Installer-consumed pin, signature/hash checks, loaded-runtime identity and patch-PR workflows implemented. Hosted enablement/PR execution and deployment remain maintainer actions. |
| Consumption, CI, packaging and licenses | Required, narrow | Independent example, package materials, inventory and CI definitions present. Verify clean supported install/update and actual artifact obligations; configure native runner. |

## Email application matrix

The email application owns these features. `email-renderer` is a reference layer
above the generic crate, not an implicit core profile or a complete email client.

| Email need | First-reader expectation | Current reference scope / application work |
| --- | --- | --- |
| MIME/message parsing and body selection | App required | Caller supplies decoded HTML/plain text and validated attachment/CID records. No MIME parser here. |
| HTML/CSS sanitization and quotas | Required | Bounded sanitizer, inline CSS allowlist and malformed/deep markup tests present. Extend real/hostile corpus and fidelity review. |
| Inline CID/data images | Required | Scoped opaque IDs and bounded static PNG normalization present; orientation applied, animation/metadata discarded. Validate MIME extraction at the app boundary. |
| Remote images blocked before consent | Required | Restricted WebKit plus placeholders present. Preserve zero HTTP/TCP fixture checks and examine additional request/hint/redirect cases. |
| Per-message remote-image consent | Required reference flow | Revocable consent, public-address/TLS/redirect policy and bounded worker fetching present. Review DNS/slow-network/cancellation and any app proxy/cache. |
| Safe links, anchors and mailto | App required | Intent/allowlist mechanisms and demo UX present. Real app must implement destination display, confirmation, local anchors and compose routing. |
| Attachment open/save | App required integration | No arbitrary file write/download route from message HTML. App must validate records, destinations and external launches. |
| Replacement and stale work | Required | Fresh document identity, resource revocation and consent checks present. App must reject late workers after switching/rebuilding messages. |
| Selection, copy-link, find and zoom | Required | Core/demo mechanisms present. Review multilingual behavior and keyboard/context UX in the adopting app. |
| Readable tables, links, fonts and images | Required | Basic inline/table layout retained. Real newsletter and long-URL/RTL/CJK/emoji corpus still needs visual review. |
| Loading, missing images, limits and retry | App required | Demo controls exist. Product UX must distinguish unavailable, blocked, failed and oversized content. |
| Host text fallback | Required | Prepared fallback text and demo GPUI path present. Make it accessible and usable when WebKit is disabled or unsafe. |
| Cache/retention/logging policy | App required | App must choose storage scope and avoid sensitive diagnostics; ephemeral webview state alone does not define retention. |
| Screen-reader access | App must account for gap | Native tree bridge deferred. Use an accessible host text path where required. |
| Rich styles/fonts/active embeds | Excluded from first reader | Keep blocked; granting consent to images must not enable general WebKit networking. |

See [email rendering](email-rendering.md) for the integration contract and
[security](security.md) for the mechanisms and their limits.

## Release work still to close

1. **Lifecycle and failure coverage:** extend close/reload/resource/fence races and
   recovery tests on the declared target. The recorded 1,000-cycle suite and 32
   constructor faults cover specific cases. The interrupted 26-minute run is partial
   evidence; the hour soak was waived for that delivery and is not a required rerun.
2. **Input and accessibility boundaries:** expand real keyboard, multilingual copy/
   find, selection capture, modal focus and clipboard failure tests. Keep incomplete
   IME and accessibility visible; don't relabel them as implemented.
3. **Performance:** keep readiness-driven servicing, asynchronous presentation,
   bounded caching and measured resize/idle/input/scroll gates in 0.1.0. Add actual
   compositor presentation correlation where supported and record unavailable stages.
4. **Graphics and geometry:** reproduce/bound the desktop resize failure, test
   physical resize/visibility/scale transitions on supported outputs, and record
   actual client/compositor renderer identities.
5. **Application security and fidelity:** review hostile markup, origin/resource
   replacement, consent networking/cancellation and newsletters. Test host fallback
   and attachment/external-link behavior in the adopting app.
6. **API and consumption:** run final-tree build/lint/tests/docs, native suites and
   standalone/upstream compile proof. Verify supported operations and unsupported
   errors against the published docs; avoid unimplemented extension types.
7. **Distribution and updates:** verify a clean supported install/update/uninstall
   route, source/runtime provenance and shipped licensing materials. Enable hosted
   automation/native runner and verify patch proposals. No automatic merge,
   vulnerability-coverage claim or fixed single-maintainer response SLA.

For a release decision, record each required check as passed, failed, untested, or
explicitly outside the published scope, linking its evidence and exact build.
Do not hide a required failure by calling it an exclusion. A previous dirty-tree
checkpoint or a passing demo is insufficient for a new release artifact.

## Later 0.x / broader v1

| Area | Desired capability |
| --- | --- |
| Accessibility / IME | Native accessibility tree, screen-reader interaction, surrounding text, accurate caret/candidate placement, composition and full native/DOM key semantics. |
| Sessions / browser controls | Persistent profiles, explicit sharing, cookies/site-data clearing, proxy/auth/client-certificate controls and restart semantics. |
| Resources / loading | Caller-selected base URIs, streaming/range responses, richer HTTP status/header handling and advanced navigation/request APIs. |
| Script / host interaction | Concurrent typed evaluation/results, initialization timing, isolated worlds, bounded origin/frame-scoped messaging and revocation. |
| Host hooks / media | Rich JS dialog, file, download, permission, capture, fullscreen and media behavior with explicit app decisions. |
| Reader layout | Intrinsic content size/auto-height, capped thread layouts, quoted-text/signature folding and sender-level remote-content rules. |
| Editing / export | Rich email composition/spellcheck, HTML/image clipboard, primary selection, drag/drop, printing/PDF/page export. |
| Diagnostics | Supported inspector/snapshot/export APIs with privacy and capability contracts. |
| Hardware / packaging | Larger GPU/compositor/output matrix, multi-GPU, software fallback, broader distribution packaging and operational recovery. |

These capabilities are not prerequisites for evaluating the current bounded reader
or consuming the experimental embedding API. They need separate contracts and
evidence before being advertised as supported.
