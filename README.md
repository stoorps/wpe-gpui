# WPE + gpui-fast browser demo

A native Wayland prototype that embeds WPE WebKit in gpui-fast's composition
surface. The toolbar, editable URL bar, tabs, menus, popovers, and modals are
GPUI. The default page is:

https://fullsphere.co.uk/html-emails/free-template/

## Build and run

Requires Linux, a native Wayland session, Rust 1.98.1 (selected automatically by
rustup), WPE WebKit **2.54+ with WPEPlatform**, EGL/GLES, Wayland development
headers, xkbcommon, libdrm, and GPUI's font/graphics dependencies. The initial
Cargo build downloads the pinned gpui-fast source and its dependencies.

This checkout already has the local WPE package prepared. Run it directly:

```sh
cargo run -p browser-demo
```

Cargo automatically discovers `.deps/` for compilation and uses a runner to
supply WPE's packaged helper paths at runtime. No shell setup is required.
When `.deps/` is absent, Cargo uses system WPE and launches the executable directly.
An existing `PKG_CONFIG_PATH` environment variable takes precedence over the
repository default.

On a fresh Arch checkout, you can install system packages:

```sh
sudo pacman -S --needed base-devel pkgconf wpewebkit wayland libxkbcommon libdrm mesa libglvnd fontconfig freetype2 ttf-noto
cargo run -p browser-demo
```

Keep the appropriate Vulkan driver installed for your GPU. A working EGL
DMA-BUF import path is required; this prototype has no guaranteed software
rendering fallback. Other distributions need equivalent development packages
and WPE 2.54 or newer.

For an alternate initial page:

```sh
cargo run -p browser-demo -- demo://editing/
```

If system installation is unavailable, the Arch-only local setup verifies the
package's signature and extracts it under `.deps/`:

```sh
./scripts/setup-local-arch.sh
cargo run -p browser-demo
```

The local launcher requires `bubblewrap` with read-only overlay support and
unprivileged user namespaces. It supplies WPE's compiled `/usr/lib` runtime
paths in a private mount namespace; it leaves system packages unchanged.
WebKit's own child-process sandbox remains enabled. The already-prepared local
`.deps/` directory can be used directly in this checkout.

## Try the interoperability controls

- Back/Forward, Reload/Stop, Home, zoom, and Enter/Go in the address bar.
- `Ctrl+L` focuses and selects the URL; `Ctrl+T` / `Ctrl+W` open/close tabs;
  `Ctrl+Tab` / `Ctrl+Shift+Tab` change tabs; `Ctrl+R` reloads.
- **File → Input interoperability lab** opens input, textarea, contenteditable,
  selection, event logging, history, and target-blank link fixtures.
- **View** opens a long GPUI dropdown across the page. Its **Overlay Test**
  submenu, popover, and test modal exercise stacking and hit-testing.
- **Test modal** opens a GPUI field above a translucent backdrop. Type/paste
  there, dismiss with Escape, and check that page editing resumes.
- Resize continuously with a menu or modal open; maximize/restore and move
  between differently scaled displays.
- **Help → Offline email fixture** loads sanitized HTML with JavaScript disabled
  and a restrictive CSP. Remote content stays blocked. Email links prompt in
  GPUI before opening a browser tab.

## Checks

Using either local or system WPE:

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo build -p browser-demo
python3 scripts/test-interop.py
```

For the locally extracted runtime, run the integration check with
`python3 scripts/test-interop.py --local-runtime`. Cargo build, test, and run
commands automatically use the repository setup, including when invoked from
a workspace member directory. The explicit `scripts/run-local.py` launcher
remains available for an already-built debug executable.
The native integration check opens a window, dispatches GPUI input, exercises
WPE, and exits. It also records requests to the email fixture's tracking
endpoint and fails if any are received. Avoid interacting with the test window
while it runs. Synthetic events cannot establish a Wayland compositor input
serial, so external clipboard ownership still needs a physical-input check.

See [verified findings and limitations](docs/prototype/findings.md) and the
[prototype plan](docs/prototype/README.md). This is an interoperability prototype,
not a production browser or a fully hardened email renderer.
