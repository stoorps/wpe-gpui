# WPE WebKit view for gpui-fast: prototype plan

Status: initial implementation available as `browser-demo`; see [findings and remaining validation](findings.md). Original plan researched 2026-10-31.

Build a reusable Linux WPE WebKit view inside a gpui-fast window, then exercise it in **`browser-demo`**, a tabbed browser with GPUI controls. The default demo URI is **https://fullsphere.co.uk/html-emails/free-template/**. The prototype should establish whether rendering, resizing, input forwarding, clipboard access, and GPUI overlays work together before reusing the view in the planned email app.

## Scope and decisions

- Start with **Linux on native Wayland**. Add X11 as a follow-up after the Wayland acceptance checks pass. WPE is the Linux implementation; macOS and Windows adapters are outside this prototype.
- Use **WPE WebKit 2.54 or newer with WPEPlatform 2.0**, pinned to a tested patch version. WPEPlatform became the default, stable integration API in 2.54; the older libwpe integration is deprecated. Do not start with WPEBackend-fdo. [WPE release announcement](https://wpewebkit.org/blog/2026-09-16-wpewebkit-2.54.html)
- Use gpui-fast's existing native composition slot, with a small custom WPEPlatform implementation and EGL presenter. This is a proposed integration, not an already-proven adapter.
- Implement the browser buttons, editable URL bar, tabs, menubar, dropdown menus, loading/error states, and triggerable modals in **GPUI**. WPE owns HTML layout, text selection, editing, links, and scrolling inside the web viewport. The browser chrome must not be HTML rendered by WPE.
- Support both a live-URI demo and a separate HTML-message mode. The latter establishes the reusable API needed by an email app.
- Deliver a standalone repository with a small library and demo binary. Do not build mailbox sync, MIME parsing, a composer, authentication, or a full email client.

## Evidence and constraints

The folder was empty at planning time. The inspected host is Arch Linux x86_64 with Rust/Cargo 1.97.0. Neither WPE pkg-config module was discoverable during inspection; packages may be missing or outside the search path. No dependencies were installed.

The inspected gpui-fast revision is [`7fcd37e6ef8caf212e484d5a8e4cd971dad690e1`](https://github.com/longbridge/gpui-fast/tree/7fcd37e6ef8caf212e484d5a8e4cd971dad690e1). Its checked-in toolchain requests Rust 1.98.1. Pin this revision for the first spike and resolve the toolchain before building. Its source workspace packages are named `gpui` and `gpui_platform`; published aliases differ, so use the names from the selected revision's manifests.

Relevant code at that revision:

- [`composition.rs`](https://github.com/longbridge/gpui-fast/blob/7fcd37e6ef8caf212e484d5a8e4cd971dad690e1/crates/gpui/src/fast/composition.rs): `Window::enable_window_composition()`, `create_native_surface()`, bounds/visibility management, input registration, and surface removal.
- [`Wayland composition`](https://github.com/longbridge/gpui-fast/blob/7fcd37e6ef8caf212e484d5a8e4cd971dad690e1/crates/gpui_linux/src/fast/composition/wayland.rs): GPUI owns the subsurface; the producer receives a raw `wl_surface` handle and must stop presenting before it is destroyed.
- [`native_webview.rs`](https://github.com/longbridge/gpui-fast/blob/7fcd37e6ef8caf212e484d5a8e4cd971dad690e1/crates/gpui_perf/examples/native_webview.rs): Linux demonstrates an independent GPU producer, rather than a Linux web engine. Reuse its surface/element arrangement, not its continuous animation loop.
- [`linux_webview.rs`](https://github.com/longbridge/gpui-fast/blob/7fcd37e6ef8caf212e484d5a8e4cd971dad690e1/crates/gpui_perf/examples/linux_webview.rs): an existing WebKitGTK/Wry example requires X11 or XWayland. It is useful reference code but does not satisfy native Wayland WPE embedding.

Importantly, `WindowComposition::on_input()` dispatches mouse and wheel events only. Keyboard, focus, and clipboard need explicit integration.

WPE exposes public display, toplevel, and view subclasses, including buffer delivery, EGL import, input delivery, and separate frame-presented/buffer-released notifications. These are the intended integration points. The built-in Wayland backend does not document adopting GPUI's existing surface, so use a custom platform instead of assuming that capability. [WPE platform implementation guide](https://wpewebkit.org/reference/stable/wpe-platform-2.0/tutorial-platform.html)

The requested URI is an HTML newsletter with images, single-column and two-column sections, links, and a footer. It is a useful layout/scrolling demonstration; it does not exercise MIME attachments or prove email safety. [Demo content](https://fullsphere.co.uk/html-emails/free-template/)

## Proposed architecture

The data flow will be:

1. A GPUI browser entity owns tab state and GPUI controls; each tab owns its WPE view controller, policy, status, history, and composition slot. Only the active tab is visible and receives input.
2. A GPUI element reserves the viewport and updates the slot during prepaint.
3. A narrow C bridge implements the WPEPlatform GObject subclasses and WebKit callbacks; a Rust wrapper owns it through opaque handles.
4. WPE sends rendered buffers to the bridge. An EGL/GLES presenter imports supported DMA-BUFs and draws into an EGL window surface backed by GPUI's native slot.
5. GPUI receives pointer events for the slot and forwards translated events to WPE. A GPUI focusable host handles keyboard events separately.
6. WPE load, navigation, cursor, and process events update the GPUI entity through queued notifications.

Use a C bridge initially to keep GObject subclassing and version-sensitive C APIs out of hand-written Rust layouts. Build it with `cc` and discover libraries with `pkg-config`. Expose create/destroy, load, geometry, visibility, focus, event, and status operations. Keep unsafe code confined to the wrapper and bridge; no Rust unwinding through C callbacks.

The custom WPE toplevel represents the allocated viewport, not another desktop window. GPUI retains ownership of the Wayland connection and surface role. The presenter borrows the display/surface handles and creates its own EGL context and `wl_egl_window`; it must not assign an xdg role, dispatch GPUI's Wayland event queue, or destroy GPUI objects. This shared-connection arrangement is the first feasibility test.

### Rendering and lifetime rules

- Negotiate only buffer formats/modifiers that the actual EGL importer supports. Verify alpha, channel order, stride, orientation, and device selection on the test machine.
- Respect rendering fences before sampling; advertise explicit synchronization only once implemented. Release a source buffer only after GPU sampling has completed. EGL swap completion alone must not be treated as proof that sampling is finished.
- Track accepted buffers through queued, submitted, presented, and released states. Ensure exactly-once completion/release on replacement, hiding, resize, failure, and shutdown. Bound the queue and document how superseded frames are completed under the pinned API.
- Investigate SHM upload as a diagnostic fallback, but do not assume WPE will supply SHM on every configuration. An unsupported DMA-BUF path must produce an actionable error rather than a stalled page.
- Apply layout bounds in device pixels to GPUI; maintain WPE logical dimensions and device scale separately. Pointer coordinates are window-logical coordinates: subtract the viewport origin before translating to WPE. Test fractional scale without applying scale twice.
- Hide/unmap WPE when its pane is hidden; defer zero-size presentation. Keep ownership independent of repeated GPUI render calls.
- On shutdown: stop new work, disconnect callbacks/input, finish outstanding GPU use, release WPE buffers and objects, destroy presenter resources, then remove/drop the GPUI composition slot.

The EGL approach avoids an initial dependency on importing external textures into GPUI's wgpu renderer. Direct DMA-BUF attachment to the subsurface or wgpu import can be evaluated later if measurements justify the extra integration work.

### Event loop and input

Keep WebKit, GLib, EGL, and the GPUI controller on the UI thread for the first prototype. Never run a blocking GLib main loop inside GPUI. Start with a bounded, nonblocking GLib context pump scheduled by GPUI, following the upstream example's general approach. Record its latency and idle wakeups. Replace timer pumping with readiness/deadline integration if feasible; do not describe a timer-driven version as idle-efficient.

Pointer capture must preserve drag selection and deliver release events outside the viewport. GPUI overlays must receive their input before the page. Route focus explicitly on click, Tab, pane changes, window deactivation, and modal opening. Deliver key-up/down, modifiers, and repeat only while the page is focused; retain application shortcuts in GPUI. Check whether GPUI exposes enough native key information for WPE keymaps; document any minimal upstream hook required.

Include keyboard scrolling, drag/keyboard selection, Select All, Copy, Cut, Paste, typing, deletion, caret movement, and undo/redo in web inputs, textareas, and `contenteditable` elements. Forward text input as well as key events, preserving keyboard layouts and avoiding duplicate character insertion. Connect GPUI text-input/composition events to WPE's input-method integration; verify preedit/commit/cancel with an IME where available and report platform limitations explicitly. Basic editable web content is required for this prototype.

The WPE clipboard must connect to the system clipboard; a platform-local clipboard is insufficient. Test copy/paste in both directions between web content, the GPUI URL bar, and another application. Dispatch Edit menu actions and shortcuts to the currently focused GPUI control or WPE view. A modal must suspend page input and restore the previous valid focus on dismissal. Tab switching must cancel stale pointer capture and prevent keys or paste operations reaching an inactive view. Accessibility bridging, touch, and drag-and-drop remain later work.

### Browser controls and overlay exercises

- **Toolbar:** Back, Forward, Reload/Stop, Home (the default URI), an editable URL bar, and zoom controls. Enable history buttons from the active view's actual history state. Enter navigates; Escape cancels URL editing; committed navigation and redirects update the displayed URL without overwriting an in-progress edit.
- **Tabs:** New Tab, activate, and Close Tab, with title and loading state. Preserve each tab's URL, history, scroll position, and editing state while switching. New tabs initially load the default URI. Closing a tab tears down its WPE resources; closing the final tab creates a fresh default tab. Keep session persistence out of scope.
- **Menubar:** File, Edit, View, Tabs, and Help. Include working items such as New/Close Tab, Reload, Copy/Cut/Paste/Select All, zoom, Next/Previous Tab, and About. Add an Overlay Test submenu under View and enough entries for the open menu to extend well below the chrome across the webview; do not keep every dropdown confined to the toolbar.
- **Triggerable modals:** an About dialog and an Overlay Test dialog opened through menu items and a visible GPUI button. The test dialog includes an editable GPUI field, buttons, a shadow, and a translucent full-window backdrop over the page. Include a popover and a submenu to exercise multiple overlay layers.
- **Overlay behavior:** verify visual stacking, translucent blending, shadows, keyboard navigation, outside-click dismissal, Escape, and focus restoration. Menu/dialog clicks, typing, and wheel events must not leak to the page underneath. Test overlays over selected text and an actively edited web field, including during resize and tab changes.

Resizing is a required interop test: continuously resize the window and its web viewport while loading, scrolling, editing, and showing overlays. Synchronize GPUI layout, native-slot bounds, EGL drawable size, WPE logical size, and device scale. Ignore or retire obsolete-size frames safely so stale buffers do not stretch, clip, or stall the page. Test maximize/restore, zero-size/hide recovery, and output scale changes; content and hit-testing must remain aligned.

## Email-content contract

Proposed public operations, with exact signatures settled during implementation:

- `load_uri(uri)` for the controlled demo.
- `load_html(html, base_uri, policy)` for a prepared email body.
- `go_back()`, `go_forward()`, `reload()`, `stop()`, and `set_zoom()`.
- Focus-aware editing commands for selection, copy/cut/paste, and undo/redo.
- Events for URL/title/history changes, loading/ready/failure, requested external links/new tabs, and web-process termination.

Use explicit policies rather than a general-purpose browser configuration:

**Browser-demo policy:** load the requested HTTPS URI by default with remote assets enabled. Allow HTTP(S) navigation from the URL bar and page links, maintain per-tab history, and reflect redirects in the GPUI controls. Route target-blank/new-window requests to GPUI-managed tabs; deny unrequested popups, unsupported schemes, downloads, and permissions. Support JavaScript for browser interoperability tests, including a local editing/event fixture; this setting must remain separate from the email policy. Use ephemeral session storage for the prototype.

**Email policy:** disable JavaScript, plugins where applicable, persistent website storage, popups, downloads, permissions, and arbitrary navigation. Use an ephemeral network session. Sanitize HTML with an established parser/sanitizer and set a restrictive CSP; never sanitize with regexes. Treat base URIs and all URLs as untrusted. Block remote resources by default, including CSS imports, fonts, frames, tracking pixels, and redirected requests, not just `<img>` URLs.

Prove where request interception occurs in the pinned WPE version. Navigation-policy callbacks alone do not block subresources. If a web-process extension is required, include it as a separate build artifact and deny requests there; do not ship a cosmetic remote-images toggle. Test resource denial with a local request-counting server before claiming it works.

Provide a controlled custom scheme for app-supplied resources, including future `cid:` attachment mapping. Restrict resource identifiers to a per-message allowlist and deny arbitrary file access and path traversal. Keep MIME parsing and attachment extraction in the future email application's layer. Scope any “load remote content” action to one message.

Keep WebKit's process sandbox enabled and TLS validation intact. Full production email security is a separate review, but policy violations found by prototype tests block declaring the email mode complete.

## Implementation sequence

### 1. Establish a reproducible build

Create a Cargo workspace, pin gpui-fast and the toolchain, add the C bridge build, and document the tested WPE/GLib/EGL/GLES/Wayland development and runtime dependencies. Verify the actual Arch package names rather than copying a generic distro command. Add startup diagnostics for session type, versions, selected renderer, and missing capabilities.

**Exit:** a GPUI window opens on native Wayland, and WPE can create a view and load a small local HTML fixture with the sandbox enabled.

### 2. Prove one embedded frame, then sustained updates

Create one GPUI native slot and the custom WPE display/toplevel/view. Establish EGL presentation, buffer negotiation, completion/release, and teardown. Begin with a local static page, then change its contents and scroll it. Test continuous resize and pane hide/show before adding the browser shell.

**Exit:** actual WPE-rendered HTML appears inside the GPUI window; updates continue without buffer starvation; repeated create/destroy is clean. If surface borrowing or synchronization fails, record the exact blocker and evaluate direct subsurface buffer presentation before expanding scope. Do not substitute a separate browser window as success.

### 3. Add interaction and composition

Implement pointer/wheel forwarding, focus/keyboard/text-input routing, cursor feedback, selection/copy/cut/paste, typing in web fields, zoom, and scale changes. Add the local editing/event fixture and a GPUI popover and modal covering the page to test ordering and input isolation. Isolate any necessary gpui-fast change in a small patch with an explanation.

**Exit:** scrolling, selection, typing, and clipboard transfers work in web content; shortcuts follow focus; overlays paint and receive input without leakage; continuous viewport resizing keeps content and hit-testing aligned without starving frame delivery.

### 4. Build `browser-demo`

Build the GPUI toolbar, editable URL bar, tab strip, menubar, dropdowns/submenus, and triggerable modals described above. Default to the exact FullSphere URI. Wire controls to actual WPE navigation, history, loading, and editing state. Include menu actions to open the local editing/event fixture and the separate HTML-message fixture. Remote-content controls apply to message mode and must visibly reflect its policy.

**Exit:** browser navigation and multiple tabs work; the newsletter renders at narrow and wide viewport widths; GPUI menus and modals overlap the page correctly; typing and clipboard operations follow focus across chrome, page, tabs, and modals. Failed loads show an error and retry action without hanging the shell.

### 5. Implement and verify email policy

Add sanitized HTML loading, resource-scheme handling, isolated session state, navigation reporting, and enforceable request blocking. Test malicious fixtures, redirects, and web-process termination/recreation.

**Exit:** default message mode makes zero external requests in the controlled network test, scripts do not execute, links are reported to the host, and switching messages does not inherit remote-content permission.

### 6. Package the findings

Document build/run instructions, tested versions and compositor/GPU, limitations, manual checks, and measurements. Record first-frame latency, resize/scroll behavior, idle wakeups, and buffer/FD counts across repeated lifecycle tests. Separate cold live-network loads from local-fixture results. Add X11 only as a separately tested extension of the presenter.

**Exit:** another developer can reproduce the demo and distinguish proven capabilities from follow-up work.

## Planned repository layout

```text
Cargo.toml / Cargo.lock / rust-toolchain.toml
crates/gpui-wpe/             # Safe Rust controller and GPUI element
crates/wpe-bridge/           # FFI, build.rs, custom WPEPlatform, EGL presenter
crates/wpe-web-extension/    # Only if needed for enforceable request policy
examples/browser-demo/      # GPUI browser controls, tabs, menus, and modals
fixtures/                   # Local layout, editing/event, and policy test HTML
docs/prototype/README.md    # This plan
docs/prototype/findings.md  # Build instructions, measurements, limitations
```

The live site remains the requested default. Add an original local newsletter-style fixture for repeatable checks without relying on the site's availability or redistributing its assets. Add an editing/event fixture with inputs, a textarea, `contenteditable`, and event logging so the newsletter is not the only input test. Include linked local pages and a target-blank link to exercise history and tabs deterministically.

## Validation and completion checklist

- Build/check the Rust workspace and C bridge; document the exact command and environment.
- Manually verify native Wayland embedding, scroll, drag/keyboard selection, typing/editing, copy/cut/paste, text composition, and focus transfer between the GPUI URL bar, web fields, and modal fields. Check clipboard exchange with another application and ensure each input event is delivered once.
- Verify Back/Forward state, URL submission and redirect updates, Reload/Stop, new/close/switch tabs, independent tab state, and cleanup of closed tabs.
- Open each GPUI menubar dropdown, a submenu, popover, and both modals over the webview. Check shadows/translucency, keyboard control, outside-click/Escape handling, focus restoration, and absence of pointer/key/wheel leakage to the page.
- Exercise narrow/wide layouts and scale factors 1.0, 1.25, 1.5, and 2.0 where the compositor supports them, including moving between outputs.
- Check hide/show, minimize/maximize/restore, continuous resize while editing/loading and displaying overlays, repeated tab open/close, and web-process termination. Verify viewport geometry and input alignment after each change, including fractional scale and zero-size recovery. Track outstanding buffers and file descriptors to catch leaks and starvation.
- Test pure event-coordinate/policy logic and the meaningful lifecycle invariants. Use local fixtures for integration tests; reserve the live URI for a smoke test.
- Use a local request recorder to test remote images, CSS imports, fonts, frames, redirects, forms, scripts, file URLs, custom-scheme traversal, and per-message permission reset.
- Record CPU/idle behavior and whether the GLib pump or presenter blocks GPUI. Do not claim zero-copy, a frame-rate target, or production readiness without evidence.

The prototype is complete when `browser-demo` displays the requested URI inside its GPUI browser shell; browser buttons, the URL bar, and tabs work; GPUI menus/modals draw over the webview and isolate input; resizing remains correct; and users can select, copy, paste, and type in web content. The separate HTML-message mode must meet its tested policy, and the reproducible findings document must explain remaining limitations. A screenshot alone is insufficient.

## API references

- [WPEWebKit 2.0 reference](https://wpewebkit.org/reference/stable/wpe-webkit-2.0/)
- [WPEPlatform 2.0 reference](https://wpewebkit.org/reference/stable/wpe-platform-2.0/)
- [WebKit graphics architecture](https://docs.webkit.org/Ports/WebKitGTK%20and%20WPE%20WebKit/Graphics.html)

Stable-reference URLs move with releases; the implementation findings should record the installed patch version and the headers actually used.
