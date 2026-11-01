# Architecture and ownership

The workspace separates application behavior, GPUI embedding, and native engine
integration. The core has no email profile, sanitizer, MIME parser, or default URI.

| Component | Owns |
| --- | --- |
| `browser-demo` | GPUI controls, tabs, menus, modals, navigation UX, recovery UX. |
| `email-renderer` | App-level HTML/CSS preparation, image normalization, consent and bounded fetching. It has no GPUI/WPE dependency. |
| `gpui-wpe` | Managed GPUI entity, settings/policy, document identity, events, geometry/input and automatic lifetime integration. |
| `wpe-bridge` | UI-thread-local Rust/C boundary, custom WPEPlatform types, GLib readiness, EGL imports, presentation, native callbacks. |

## Composition and rendering

GPUI creates and owns the native Wayland composition slot. WPE produces content
buffers through a custom WPEPlatform display/toplevel/view implementation. A GLES
presenter imports those buffers and renders into the slot's EGL window surface.
GPUI draws its overlay content above that surface.

Layout/prepaint propagates the logical document rectangle, independent clip, output
scale, and visibility. Keeping clip separate from document size avoids reflow when
only part of the view is exposed. User zoom is separate from output scale. The
renderer uses a bounded viewport rather than allocating a surface as tall as an
entire email or web document.

WPE owns and terminates the EGLDisplay returned by its platform implementation.
It therefore receives a separate device/surfaceless EGL display. Presenters borrow
GPUI's Wayland EGL display and must never terminate it. Sharing the borrowed display
with WPE ownership caused cross-view invalidation in the original implementation;
the separate ownership domains are essential.

Incoming buffers wait asynchronously for acquire readiness. Completion fences
separate GPU sampling from source release. At most four pending frames and eight
resident imports per live view bound queue/cache growth. Imports are cached with
owned lifetime information and evicted only when safe. The normal render path has
no unconditional `glFinish()` or host-driven 16 ms polling loop.

An exceptional post-sampling fence-allocation failure can require synchronous
completion to establish safe release. Failure to establish GPU completion during
catastrophic driver/fence failure is not advertised as graceful recovery; preserving
buffer ownership can require stopping the process. The tested constructor-failure
cases do not prove arbitrary GPU-loss handling.

## Thread and event-loop model

GPUI, WPE, GLib dispatch, EGL operations, and view callbacks belong to the owning UI
thread. The library captures an owned readiness snapshot, duplicating descriptors
before a worker waits for readiness/deadlines. The worker signals the UI executor;
it never dispatches the native GLib context or accesses a native view.

A bounded UI pump services native work. Driver generations prevent stale wakeups
from acting on a replacement driver. Closing the last view retires native ownership;
library hooks handle app quit and OS-window closure. Do not add an independent
worker-side GLib loop or a second periodic host pump.

## View and surface lifetime

The native producer must stop before GPUI releases its composition slot and before
the Wayland display disconnects. Entity destruction alone is insufficient: GPUI
frames or another host can retain an entity after its window closes.

The core registers window-close and app-quit hooks, closes views deterministically,
revokes pending work, and releases composition ownership in the correct order.
Close is idempotent. Retained entities report closed state and stop native input
and presentation. `WebViewHandle` stores weak ownership and returns `Closed` once
invalidated. Creation is rejected during app shutdown.

## Document and resource lifetime

Each `Document` has a generated ID and custom-scheme namespace. Resource access
is scoped to that ID. Native load validation is transactional: invalid replacement
input must not partially install a new document/resource identity. Different HTML
cannot reuse an identity that has already been loaded.

Asynchronous responses carry view instance, document, and request-generation
information. Replacement, reload, close, failure, and deadline paths invalidate
obsolete work. The worker-safe responder settles once and fails on drop; native
pointers stay on the UI thread. History retains a bounded set of document/resource
snapshots, with identity synchronized after native navigation.

Restricted views wait for a shared constant HTTP(S) content filter before loading.
Only the newest pending document is retained. Compiler callbacks use owned job
state and live-view lookup rather than a raw view pointer. Closing a waiter removes
its eligibility. See [security](security.md) for fail-closed behavior and limits.

## Shared engine constraints

Views have ephemeral native network sessions, but they share WPE engine/platform
infrastructure. This is not a process-per-view guarantee. WPE's primary display and
clipboard are shared, as is its native appearance preference. Those constraints
explain the selected-text query and per-view theme limits in [support](support.md).
Do not infer privacy or isolation from separate GPUI entities alone.

For measurement endpoints and retained failure evidence, see [performance](performance.md)
and the [evidence index](evidence/README.md).
