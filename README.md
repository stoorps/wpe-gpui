# gpui-wpe

Embed [WPE WebKit](https://wpewebkit.org/) in [GPUI Fast](https://github.com/longbridge/gpui-fast) applications on native Linux Wayland.

`gpui-wpe` provides a GPUI `WebView` entity for web pages and application-supplied
HTML. Web content renders into GPUI's composition surface, so your application
can draw GPUI menus, popovers, and modals over it. The library manages native
lifecycle, event-loop integration, rendering, and input forwarding.

The core is workload neutral. It exposes settings, navigation decisions, and
resource policies; applications own their browser UI, content sanitization, and
workload-specific security rules. The separate `email-renderer` crate demonstrates
that application layer for an email reader.

**Status:** experimental 0.1.0 candidate, with an evolving API and a narrow tested
platform scope. Full accessibility, full IME support, software rendering fallback,
and broad GPU/compositor coverage are not available. See the
[capabilities and limitations](docs/support.md)
before choosing it for an application.

## What it provides

- A managed `WebView` entity with a builder API shaped around `gpui-webview`.
- URL and HTML loading, navigation/history, load state, typed events and errors.
- Pointer, scrolling, keyboard, focus, selection, clipboard, find, and zoom APIs.
- Viewport resizing, clipping, scaling, visibility, and GPUI overlay composition.
- Document-scoped resources, bounded asynchronous resource delivery, and explicit
  navigation and network policies.
- Library-owned shutdown and readiness-driven GLib integration, asynchronous GPU
  fences, and a bounded buffer-import cache.

The API shares GPUI types and construction conventions with `gpui-webview`; it is
not a drop-in Wry backend or a promise of cross-platform feature parity. This
implementation targets Linux Wayland. Windows and macOS applications can use
upstream `gpui-webview` through an application-owned adapter.

## Run the demo

The tabbed `browser-demo` uses GPUI for its toolbar, URL bar, tabs, menus, and
modals. Its default page is [FullSphere's newsletter template](https://fullsphere.co.uk/html-emails/free-template/).

You need a native Wayland session, Rust **1.98.1** (selected by `rust-toolchain.toml`),
WPE WebKit **2.54+ with WPEPlatform**, EGL/GLES, and GPUI's graphics/font dependencies.
A working EGL DMA-BUF import path is required. The recorded reference runtime is
WPE WebKit 2.54.2 on Linux x86_64; other runtime/driver combinations need validation.

On Arch Linux, install the native build dependencies:

```sh
sudo pacman -S --needed base-devel pkgconf wpewebkit wayland libxkbcommon libdrm mesa libglvnd fontconfig freetype2 ttf-noto
cargo run -p browser-demo
```

Keep the appropriate Vulkan driver installed for your GPU. Other distributions
need equivalent development packages and the required WPE version.

With [just](https://github.com/casey/just) installed:

```sh
just run                         # Live default page
just run 'https://example.org/'   # Another URL
just lab                         # Local typing, selection, and clipboard lab
cargo run -p embedding-example   # Minimal host without browser controls
```

In the demo, use **View** to open menus and overlay tests, or **Help → Offline
email fixture** for the app-level sanitized reader. Resize with a menu or modal
open to exercise composition and input routing. `Ctrl+L` focuses the address bar;
`Ctrl+T` / `Ctrl+W` open and close tabs.

### Optional local WPE runtime

For Arch development without a system WPE installation, `just setup-local`
verifies the package signature and extracts the pinned runtime into `.deps/`.
Cargo discovers it automatically; no shell setup is needed. Compilation requires
`pkg-config` to find `wpe-webkit-2.0.pc`. An existing `PKG_CONFIG_PATH` takes
precedence over the repository default.

The local runner requires `bubblewrap` with read-only overlay support and
unprivileged user namespaces. It supplies the package's helper paths in a private
mount namespace while retaining WebKit's child-process sandbox. This is a
development convenience; applications must choose their own runtime distribution
and update route. Without `.deps/`, Cargo uses system WPE.

## Embed a view

Use the same GPUI dependency identity as this workspace: **gpui-kit 0.8.1** with
its `gpui-fast` feature, backed by **gpui-fast / gpui-fast-platform 0.1.12**.
For a local checkout, the relevant application dependencies are:

```toml
[dependencies]
gpui = { package = "gpui-kit", version = "=0.8.1", default-features = false, features = ["gpui-fast"] }
gpui-wpe = { path = "../wpe-gpui-fast/crates/gpui-wpe" }
```

Inside an existing GPUI window:

```rust
use gpui::{App, AppContext as _, Entity, Window};
use gpui_wpe::{WebView, WebViewBuilder};

fn create_view(window: &mut Window, cx: &mut App) -> gpui_wpe::Result<Entity<WebView>> {
    let view = WebView::build(
        WebViewBuilder::new().with_html("<h1>Hello from WPE</h1>"),
        window,
        cx,
    )?;
    Ok(cx.new(|_| view))
}
```

Render the entity with `div().child(view.clone())` in your host's `Render`
implementation. The library owns native servicing and teardown; the host does
not run a polling loop. See the [complete embedding example](examples/embedding/src/main.rs)
and [GPUI compatibility notes](docs/compatibility.md).
Generate local API documentation with `cargo doc -p gpui-wpe --no-deps --open`.

### Rendering untrusted content

Defaults use normal web-engine networking with JavaScript enabled. Loading HTML
does not sanitize it. For restricted application documents, configure
`JavaScript::Disabled` and `ResourcePolicy::ProvidedOnly` before construction,
supply document-scoped resources, and handle navigation decisions in the host.
The restricted path installs a native HTTP(S) content filter before loading;
policy initialization failures fail closed.

Email sanitization, CID handling, image validation, remote-image consent, and
plain-text fallback live in the separate [email consumer](docs/email-rendering.md).
Treat it as a reference integration that needs review against your application's
threat model. Ordinary copy/paste works in mixed sessions; the asynchronous
selected-text query has stricter session requirements. Per-view native color-scheme
overrides are unsupported. These and other limits are detailed in the
[support guide](docs/support.md).

## Workspace and documentation

| Path | Purpose |
| --- | --- |
| [`crates/gpui-wpe`](crates/gpui-wpe) | Public GPUI embedding API |
| [`crates/wpe-bridge`](crates/wpe-bridge) | Low-level Rust/C WPEPlatform and EGL integration |
| [`crates/email-renderer`](crates/email-renderer) | Application-layer email preparation and consent example |
| [`examples/browser-demo`](examples/browser-demo) | GPUI browser controls and interoperability lab |
| [`examples/embedding`](examples/embedding) | Minimal consumer, compatibility proof, and native diagnostics |

Start at the [documentation index](docs/README.md) for setup, embedding, API,
architecture, security, and testing guides. The [support matrix](docs/support.md)
describes current capabilities and limitations; the [roadmap](docs/roadmap.md)
tracks unfinished work. See the [email integration guide](docs/email-rendering.md),
[performance methodology](docs/performance.md), and
[isolated Wayland diagnostics](docs/isolated-wayland.md) for those workflows.

## Development

Run `just` to list all commands. The main checks are:

```sh
just check                  # Format, strict Clippy, Rust and Python tests; no GUI
just smoke-local            # Deterministic local rendering and clean-exit check
just test-interop           # Native input, overlays, navigation, resource policy
just test-native-policy     # Policy readiness, cancellation, fail-closed behavior
just perf-resize            # Resize correctness and submission latency
just perf-runtime           # Idle/input/scroll proxies and 1,000 lifecycle cycles
just licenses               # Dependency license inventory and notice collection
```

Native checks open Wayland windows; avoid interacting with them during a run.
For a private compositor, follow the [isolated Wayland guide](docs/isolated-wayland.md).
Performance reports distinguish submitted frames and DOM observations from actual
compositor presentation. Cross-application clipboard ownership also requires a
real desktop check. Recorded results and their scope live in the
[verification summaries](docs/evidence/README.md).

## WebKit updates

System-runtime users receive WebKit updates through their distribution. A local
`.deps/` runtime takes precedence and must be updated separately. `runtime_info()`
reports the loaded engine version and library path.

Dependabot proposes Cargo/Actions updates; the native-runtime workflow checks Arch
package availability and proposes changes to the installer pin. These are reviewable
patches, with no auto-merge or guaranteed human response window. See the
[automation setup and update instructions](.github/README.md) to enable hosted jobs
and validate native updates.

## License and attribution

Project code is [Apache-2.0](LICENSE-APACHE). Preserved upstream notices are in
[NOTICE](NOTICE); dependency licenses, source provenance, and native distribution
requirements are recorded in [THIRD_PARTY.md](THIRD_PARTY.md).

Built on [GPUI Kit](https://github.com/longbridge/gpui-kit),
[GPUI Fast](https://github.com/longbridge/gpui-fast), and the work of
[Zed/GPUI contributors](https://github.com/zed-industries/zed), including the adapted
demo text input. [WPE WebKit](https://wpewebkit.org/) supplies the native engine.
[gpui-webview](https://github.com/longbridge/gpui-kit/tree/main/crates/webview) and
[Wry](https://github.com/tauri-apps/wry) inform API compatibility;
[Weston](https://gitlab.freedesktop.org/wayland/weston) supplies optional diagnostic
tools. [FullSphere](https://fullsphere.co.uk/html-emails/free-template/) supplies the
live demo content; its assets are not redistributed here.

Dependencies retain their own licenses, including WebKit's component LGPL terms
and the email consumer's MPL-2.0 `cssparser` dependency. Binary distributions need
the notices and source materials applicable to the actual shipped artifacts.
