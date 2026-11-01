# Getting started

Run commands from the repository root. The examples require a native Wayland
session; X11 and XWayland are not supported by this backend.

## Requirements

- Linux; the reference target is x86_64.
- Rust 1.98.1, rustfmt, and Clippy, selected by `rust-toolchain.toml`.
- WPE WebKit 2.54 or newer with WPEPlatform. The recorded runtime is 2.54.2.
- EGL/GLES, Wayland, xkbcommon, DRM, graphics drivers, and GPUI font dependencies.
- Python 3 for tooling; `just` is optional but supplies the documented shortcuts.

The presenter requires a working EGL DMA-BUF import path. There is no guaranteed
software fallback. Install the Vulkan driver appropriate to the GPU for GPUI.
See [support](support.md) for the limits of the tested configurations.

## System runtime

On Arch Linux:

```sh
sudo pacman -S --needed base-devel pkgconf wpewebkit wayland libxkbcommon libdrm mesa libglvnd fontconfig freetype2 noto-fonts
cargo run -p browser-demo
```

Other distributions need equivalent development packages and a sufficiently
recent WPEPlatform-enabled WPE. Check discovery before building:

```sh
pkg-config --modversion wpe-webkit-2.0 wpe-platform-2.0
```

Cargo downloads the locked Rust dependencies. See [compatibility](compatibility.md)
before selecting GPUI dependencies in a consuming application.

## Local Arch development runtime

If system WPE installation is unavailable:

```sh
just setup-local
cargo run -p browser-demo
```

The installer reads `native-runtime.json`, verifies the archive hash and independent
Arch package signature, and extracts into `.deps/`. It stages replacement contents
to avoid retaining obsolete package files. The system still needs the other native
dependencies, fonts, drivers, and package-signature verification tools.

The local launcher needs `bubblewrap` with read-only overlay support and
unprivileged user namespaces. It supplies WPE's compiled helper paths through a
private `/usr/lib` overlay; WebKit's child-process sandbox remains enabled.
This mechanism is a development runtime route, not an application packaging format.

Cargo automatically searches `.deps/usr/lib/pkgconfig` and uses its Linux runner.
No sourced environment is needed. An existing `PKG_CONFIG_PATH` takes precedence
over the repository default. Without `.deps/`, the runner uses system WPE. With a
local runtime present, it takes runtime precedence; update it separately from system
packages. See [runtime updates](runtime-updates.md).

## Examples

```sh
just run                         # Default live newsletter
just run 'https://example.org/'   # Another URL
just lab                         # Offline input/selection/clipboard fixture
cargo run -p embedding-example   # Minimal embedding host
just smoke-local                 # Local render and clean-exit check
```

Without `just`, `cargo run -p browser-demo -- demo://editing/` opens the lab.
The default live URI is
[FullSphere's newsletter](https://fullsphere.co.uk/html-emails/free-template/).
It depends on the remote site's availability and behavior; use the local fixture
to distinguish installation failures from remote-page failures.

The browser demo's buttons, URL field, tabs, menus, popovers, and modals are GPUI.
Use **View** for overlay tests and **Help → Offline email fixture** for the
application-level sanitized reader. Try typing in a modal, dismissing it, and
resuming page editing. `Ctrl+L` selects the address, `Ctrl+T` / `Ctrl+W` open/close
tabs, `Ctrl+Tab` changes tabs, and `Ctrl+R` reloads.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| `wpe-webkit-2.0` not found | Install development files or run the local setup; inspect `PKG_CONFIG_PATH` and the module version. Do not point it at an incompatible WPE installation. |
| Missing helper executable or library | Use Cargo's runner for `.deps/`; check bubblewrap support and the installed runtime. Setting only `LD_LIBRARY_PATH` does not supply all helper paths. |
| Wayland display or composition error | Launch from a native Wayland session with valid `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR`; verify GPUI's graphics setup. |
| EGL/import failure | Record actual driver/runtime details and the typed failure; this backend cannot promise software fallback. |
| Live page renders but never completes loading | Compare `just smoke-local`; a frame or title alone is not successful live-page completion. |
| Clipboard works in synthetic tests only | Test real cross-application ownership with physical desktop input; see [testing](testing.md). |

For native diagnostics, use [isolated Wayland](isolated-wayland.md). Keep trace
logging separate from performance measurements.
