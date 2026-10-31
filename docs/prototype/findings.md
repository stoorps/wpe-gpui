# Prototype findings

Implemented 2026-10-31. See the root [README](../../README.md) for build/run commands.

## Implemented

- `wpe-bridge`: a UI-thread-only Rust wrapper around custom WPEPlatform display,
  toplevel, view, and input-method types. A GLES presenter imports DMA-BUF frames
  into an EGL window surface backed by GPUI's existing Wayland slot. SHM buffers
  have a pixel-upload path, but software rendering is not guaranteed.
- `gpui-wpe`: a reusable GPUI entity/element with layout-to-native geometry,
  pointer/wheel forwarding, drag capture, keyboard/editing actions, committed
  text/preedit forwarding, focus/visibility, and clipboard transfer.
- `browser-demo`: GPUI browser buttons, editable URL bar, tabs, per-tab history,
  menus, submenu, popover, About dialog, and editable test modal. Only the active
  tab is mapped and receives input. Chrome remains GPUI rather than webpage HTML.
- Browser HTTP(S) navigation, user-initiated target-blank routing, ephemeral
  sessions, permission/download denial, title/loading/history status, and errors.
- Local input/history fixtures and a separate sanitized email fixture. The
  email mode disables JavaScript, prepends a restrictive CSP, denies remote
  subresources, and prompts before opening links in a browser tab.

## Tested environment

Arch Linux x86_64, COSMIC on native Wayland, AMD/Mesa graphics, WPE WebKit 2.54.2-1,
Rust 1.98.1. gpui-fast is pinned to
`7fcd37e6ef8caf212e484d5a8e4cd971dad690e1`; Cargo.lock pins the Rust dependency graph.
WPE was extracted locally from a signature-verified Arch package. No system
packages were installed; the local launcher supplies its compiled runtime paths.

The FullSphere URI loaded with title **Newsletter**, produced embedded frames,
and reported no WebKit load or presentation error in the smoke run.

`cargo test --workspace` and strict workspace Clippy passed. The native
integration runner passed checks for:

- Rendering the input fixture.
- Typing through GPUI's actual event dispatcher and pasting through WPE's native
  editing/clipboard API, with the resulting input value read back from WebKit.
- Selecting/copying web text into the host clipboard bridge.
- Typing in the GPUI modal field without modifying the web field underneath.
- Resizing GPUI and checking WebKit's viewport width after compositor/layout
  updates settle, with a GPUI dropdown open.
- Loading a second tab, preserving the first tab's edited document across
  switches, and closing the second tab without breaking the remaining renderer.
- Back/Forward navigation and history availability.
- Loading the sanitized offline message without a render/load error.
- Zero requests to the controlled email tracking endpoint.

The runner uses synthetic input rather than a physical Wayland keyboard/pointer.
COSMIC requires a compositor-issued input serial to claim system clipboard
ownership. Consequently, inter-application clipboard exchange is implemented but
not established by this automated test. Copy/paste involving another application
must be checked using real input. The test's paste source is supplied directly to
the native clipboard adapter to avoid a false claim about compositor ownership.

## Important ownership findings

WPEPlatform owns and terminates the EGLDisplay returned by its platform
implementation. The engine therefore gets an isolated device/surfaceless EGL
display. Presenters borrow GPUI's Wayland EGL display and never terminate it;
they import DMA-BUF plane descriptors into their own presentation contexts.
Sharing the borrowed display with WPE's ownership initially invalidated other
tabs on close and caused an exit crash. One isolated engine display is shared
across tabs for consistent format/device and clipboard state.

The demo explicitly closes all presenters and removes/releases their GPUI
composition handles in the synchronous part of `App::on_app_quit`, then calls
`gpui_wpe::shutdown()`. GPUI can retain entities from old frames beyond window
removal, so waiting for entity drops is insufficient: a retained surface's
Wayland destructor would otherwise run after the client is gone. Native close
is idempotent and retained hidden elements skip input/presentation. The quit
hook also handles a window that has already closed. Embedders must follow this
shutdown order.

Buffer sampling completes with `glFinish()` before WebKit is told the source can
be reused. This favors correctness over throughput. The presenter uses a bounded
fence wait and advertises no explicit-sync support. There is no zero-copy or
frame-rate claim. EGL import formats/modifiers are queried from the local driver.
WPE's preferred-format vfunc result needs an owned reference because the base
implementation adopts it; returning only the adapter's reference caused a
teardown warning and has been corrected.

## Limits and remaining manual checks

- Only native Wayland is implemented. X11, macOS, and Windows are not included.
- Automated tests exercise modal input isolation and menu-time resizing, but do
  not prove shadows/translucency visually. Check menu/submenu/popover stacking,
  backdrop blending, outside clicks, Escape, and focus restoration by hand.
- Fractional/mixed-DPI outputs, continuous physical resize, minimize/maximize,
  process termination/recovery, prolonged tab churn, and FD/buffer leak counts
  still need manual or extended soak testing. Initial rendering is tested on
  this GPU/compositor only.
- Text/preedit/commit and basic editing are wired. Surrounding-text queries and
  exact web-caret geometry are not bridged; IME candidate positioning currently
  uses the viewport origin. Real IME sessions and non-US keyboard layouts remain
  unverified. Printable characters use the text-input path, so DOM key events
  need further work for keyboard-heavy web apps. GPUI fields support basic
  selection/editing but not a full editor's undo stack.
- Clipboard transfer currently covers plain text. Rich HTML/images, primary
  selection, drag-and-drop, accessibility bridging, and touch are future work.
- A bounded GLib pump runs every 16 ms. No idle-efficiency, latency, memory, or
  scroll-performance measurements have been completed. Native GL synchronization
  can block the UI; future work should replace conservative waits with fences.
- The offline email mode uses Ammonia plus a CSP with `default-src 'none'`, data
  images only, and no forms/frames/objects/base URI. The controlled request test
  covers its fixture, not every possible malicious message. It is not a complete
  production email security review. Sanitization can remove email styling.
- There is no remote-content opt-in, attachment/CID resource mapping, MIME parser,
  arbitrary file access, persistence, network request extension, or inspector UI.
  A request extension is unnecessary for the current strict offline policy; it
  must be reconsidered before adding selectively allowed remote resources.
- Diagnostics count successfully submitted frames; they do not timestamp actual
  compositor presentation. `evaluate()` is a trusted diagnostic API, not an
  interface to expose to untrusted messages.

## Cargo launch setup

Cargo now automatically searches the optional `.deps/usr/lib/pkgconfig` directory
and uses a Linux runner to launch the actual Cargo executable with the local WPE
runtime. `cargo run -p browser-demo` and `cargo test --workspace` need no sourced
environment. Without the local package, the runner executes against system WPE.
The runner supports test executables and custom build directories as well as the
default demo binary.
