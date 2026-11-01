# Third-party attribution and distribution notes

The project's Rust packages declare **Apache-2.0**. Keep that permissive license
for this project's code and preserve the upstream notices in
[LICENSE-APACHE](LICENSE-APACHE), [NOTICE](NOTICE), and source headers. No upstream
code or dependency is relicensed by this repository. In particular, the license
of an application package containing WPE must account for WPE's components, not
just the license of this Rust wrapper.

## Source included or adapted in this repository

| Origin | Use | License and notices |
| --- | --- | --- |
| [GPUI Fast source at the original prototype revision](https://github.com/longbridge/gpui-fast/tree/7fcd37e6ef8caf212e484d5a8e4cd971dad690e1) | Source provenance and native composition implementation references. `examples/browser-demo/src/text_input.rs` directly adapts `crates/gpui/examples/input.rs`. The current runtime uses published dependencies listed below, not this old git pin. | Apache-2.0. Copyright 2022–2025 Zed Industries, Inc.; Copyright 2026 Longbridge. Full upstream notices and license are retained in `LICENSE-APACHE`; the adaptation and modifications are recorded in `NOTICE` and its source header. |
| [Zed / GPUI](https://github.com/zed-industries/zed) | Original source underlying the GPUI code imported by GPUI Fast. The inspected GPUI Fast `UPSTREAM` file identifies Zed revision `08cbb88a152c3751a43e853425a70e9f631a5918`. | Retain the GPUI source's Apache-2.0 notices. This attribution concerns the GPUI code used here; it is not a statement about the license of every part of the Zed repository. |

Do not remove the adaptation notice when moving or extracting the text field.
Further copied source must carry its applicable license and attribution; API
compatibility or a rewrite does not justify deleting existing notices.

## Direct Rust dependencies

This inventory was refreshed from `cargo metadata --locked --all-features
--filter-platform x86_64-unknown-linux-gnu` for the implemented workspace. Versions
below identify the audited `Cargo.lock` snapshot, not claims that these are the
latest available releases. Optional upstream compatibility dependencies are shown
separately from the normal WPE path. This is a direct-dependency inventory rather
than a complete release artifact notice bundle.

Where alternatives are offered, preserve the declared SPDX choice. Project code
remains Apache-2.0; it does not make every linked component Apache-2.0 or MIT.

| Dependency / repository | Audited version / use | Declared license |
| --- | --- | --- |
| [`gpui-kit`](https://github.com/longbridge/gpui-kit) | 0.8.1; workspace alias `gpui`, Fast-backed facade | Apache-2.0 |
| [`gpui-fast`](https://github.com/longbridge/gpui-fast) | 0.1.12; Kit backend and direct embedding type-identity proof | Apache-2.0 |
| [`gpui-fast-platform`](https://github.com/longbridge/gpui-fast) | 0.1.12; workspace alias `gpui_platform` | Apache-2.0 |
| [`anyhow`](https://github.com/dtolnay/anyhow) | 1.0.104; error integration | MIT OR Apache-2.0 |
| [`libc`](https://github.com/rust-lang/libc) | 0.2.191; native bridge event-loop integration | MIT OR Apache-2.0 |
| [`raw-window-handle`](https://github.com/rust-windowing/raw-window-handle) | 0.6.2; platform handles | MIT OR Apache-2.0 OR Zlib |
| [`cc`](https://github.com/rust-lang/cc-rs) | 1.10.0; build dependency | MIT OR Apache-2.0 |
| [`pkg-config`](https://github.com/rust-lang/pkg-config-rs) | 0.3.34; build dependency | MIT OR Apache-2.0 |
| [`env_logger`](https://github.com/rust-cli/env_logger) | 0.11.11; demo logging | MIT OR Apache-2.0 |
| [`serde_json`](https://github.com/serde-rs/json) | 1.0.154; demo/test result serialization | MIT OR Apache-2.0 |
| [`unicode-segmentation`](https://github.com/unicode-rs/unicode-segmentation) | 1.13.3; demo text field | MIT OR Apache-2.0 |
| [`ammonia`](https://github.com/rust-ammonia/ammonia) | 4.2.4; application email sanitization | MIT OR Apache-2.0 |
| [`base64`](https://github.com/marshallpierce/rust-base64) | 0.22.1; application resource decoding | MIT OR Apache-2.0 |
| [`cssparser`](https://github.com/servo/rust-cssparser) | 0.38.0; application CSS policy parsing | **MPL-2.0** |
| [`html5ever`](https://github.com/servo/html5ever) | 0.29.1; application tokenizer preflight | MIT OR Apache-2.0 |
| [`image`](https://github.com/image-rs/image) | 0.25.10; application image validation/decoding | MIT OR Apache-2.0 |
| [`kuchikiki`](https://github.com/brave/kuchikiki) | 0.8.8-speedreader; application alias `kuchiki`, HTML parsing | MIT |
| [`percent-encoding`](https://github.com/servo/rust-url) | 2.3.2; application URI handling | MIT OR Apache-2.0 |
| [`reqwest`](https://github.com/seanmonstar/reqwest) | 0.12.28; application remote-image fetcher, rustls feature | MIT OR Apache-2.0 |
| [`url`](https://github.com/servo/rust-url) | 2.5.8; application URI validation | MIT OR Apache-2.0 |
| [`gpui-webview`](https://github.com/longbridge/gpui-kit/tree/main/crates/webview) | 0.8.1; optional `embedding-example/upstream-compat` dependency | Apache-2.0 |
| [`lb-wry` / Wry](https://github.com/tauri-apps/wry) | 0.53.3; alias `wry`, same optional compatibility proof | Apache-2.0 OR MIT |

The new `email-renderer` is application-level preparation/consent code, licensed
Apache-2.0 by this project. It uses the dependencies above rather than copying their
parser, sanitizer, decoder, or HTTP implementations into this repository. Its
Apache license does not replace their licenses. In particular, `cssparser` is
MPL-2.0, not a dependency that can be arbitrarily relabeled MIT/Apache.

The resolved all-feature Linux graph also contains MPL-2.0 `cssparser` 0.29.6,
`cssparser-macros` 0.6.1/0.7.1, `dtoa-short` 0.3.5, `selectors` 0.24.0, and
`option-ext` 0.2.0. Retain their applicable MPL license/source materials in release
packaging; they are not covered by this project's Apache license. The graph also
contains combined license expressions and Unicode, certificate-data, BSD, ISC,
and other notice requirements. Do not flatten an `AND` expression into an `OR`.
For alternatives such as `self_cell`'s `Apache-2.0 OR GPL-2.0-only`, a permissive
Apache branch is available; do not describe the mere presence of that expression
as requiring GPL for every consumer.

Transitive dependencies, optional features, target-specific code, and vendored
native components can add other licenses. Generate an inventory and preserve the
required license texts/notices/source materials for the exact locked dependency
graph, target, features, and shipped files before distributing binaries. Metadata
is an input to that process, not a substitute for the applicable license files or
a guarantee of complete legal compliance. Re-run the inventory when the lockfile
or target/feature selection changes.

## Reproducing the Cargo inventory

Run `just licenses` to write `target/license-inventory.json`, a readable Markdown
report, and `target/third-party-notices/`. The script uses `cargo metadata
--locked --all-features --filter-platform x86_64-unknown-linux-gnu`, records the
lockfile hash, enabled features, declared license expressions, package sources and
versions, and copies discovered package-root `LICENSE`/`LICENCE`, `COPYING` and
`NOTICE` files plus a declared `license_file`. Collected bytes have SHA256 hashes
and package provenance. Repeating the command replaces only its own unchanged
generated collection; it refuses unexpected or user-modified files.

For a selected Cargo dependency closure, use for example:

```sh
python3 scripts/license-inventory.py --package gpui-wpe --offline \
  --output target/gpui-wpe-licenses.json --markdown target/gpui-wpe-licenses.md
```

Use `--target`, `--features`, `--all-features` and `--no-default-features` to record
the intended metadata selection. Cargo's resolved features can still be unified
across workspace members; package selection narrows the graph, not Cargo's feature
resolution. Build/dev dependencies are included, so this is not the exact linked
binary inventory. Review missing metadata/files explicitly; a published crate may
omit license files from its package even when its source repository has them.
The initial all-feature Linux run found 713 packages and flagged 43 packages with
no discovered contained notice files. These counts are a checkpoint, not a fixed
release total or legal finding.

This collector deliberately does not download extra upstream files, infer license
obligations/compatibility, or inventory the WPE/WebKit native runtime, bundled
components, fonts or assets. Supply the additional materials required by the
actual release. The reusable `gpui-wpe` and `wpe-bridge` source packages each carry
their own README and an unchanged copy of the project's Apache license; package
listing checks confirm those files are included.

## Native engine and platform libraries

[WPE WebKit](https://wpewebkit.org/) is the native engine; its source is part of
[WebKit](https://github.com/WebKit/WebKit). The bridge links to the installed
WPEPlatform/WebKit APIs and system EGL/GLES, Wayland, GLib/GObject/GIO, xkbcommon,
and DRM libraries. Those projects and their contributors retain their own
copyrights and licenses.

WPE WebKit is **not an Apache-only dependency**. Its distribution contains
multiple component licenses, including BSD-style, LGPL, and other third-party
terms. The inspected Arch package's license bundle includes the WebCore LGPL
license texts and numerous bundled-component notices. Preserve the exact package's
license bundle when redistributing that runtime; do not replace it with this
project's `LICENSE-APACHE` or infer artifact licensing from one package metadata
expression.

The optional Arch development setup extracts a signature-verified package into
`.deps/`, which is excluded from Git. The package includes
`usr/share/licenses/wpewebkit/LICENSE`. That file and the package's source/build
provenance must remain available for any release process that redistributes the
runtime. Merely excluding `.deps/` from Git does not satisfy the obligations of a
binary bundle assembled from its contents.

For a bundled/runtime release, collect the actual shipped components' licenses,
notices, source/modification information, and any applicable LGPL
replacement/relinking materials or mechanisms. Record the corresponding source
and build provenance for the precise runtime build. A system-package deployment
and a bundled runtime have different shipped-file inventories; review the actual
release package rather than assuming dynamic linking alone settles every
obligation. Keeping the engine dynamically linked avoids unnecessarily folding
native runtime code into the wrapper's source distribution, but does not remove
its licenses.

## Compatibility and implementation references

- [GPUI Kit / `gpui-webview`](https://github.com/longbridge/gpui-kit/tree/944803984cf0ba3111861d6e1aa9d6265c109ff9/crates/webview),
  Copyright 2024–2026 Longbridge, Apache-2.0, supplied the inspected API/ergonomics
  baseline. GPUI Kit is now the normal facade dependency. `gpui-webview` 0.8.1 is
  additionally used by the optional `embedding-example/upstream-compat` compile
  proof; it is not required by `gpui-wpe`'s normal WPE Linux dependency path.
- [Wry](https://github.com/tauri-apps/wry) and `lb-wry` are compatibility references;
  `lb-wry` 0.53.3 is an optional compile-proof dependency with Apache-2.0 OR MIT
  licensing. Its native objects and dependencies retain their own terms. Matching
  method names does not imply Wry code was relicensed or WPE implements every Wry
  capability.
- The newly written application-level email code and WPE adapter refactoring are
  project contributions; the directly adapted text field keeps its earlier source
  provenance. Further copied upstream code requires its own preserved notices.
- WPE's public API headers and documentation informed the custom platform bridge.
  This repository does not vendor WebKit's implementation or its header files.

## Demo content and fixtures

The default live newsletter URI is provided by
[FullSphere](https://fullsphere.co.uk/html-emails/free-template/). Its website,
template, and image assets are not included in this repository and are not
relicensed under Apache-2.0 by this project. The local HTML fixtures are original
test content rather than a redistributed copy of that newsletter. Obtain the
appropriate permission/license before bundling external demo assets in a release.


## Optional compositor test tooling

[Weston](https://gitlab.freedesktop.org/wayland/weston) 15.0.1 (MIT) supplied the
isolated headless compositor and screenshooter for the recorded native tests.
It is a development/test dependency loaded from the system or a separately
signature-verified package; no Weston source or binary is vendored here. Retain
its package notices if redistributing that optional test environment. Captures
stay in ignored test artifacts; the compact checked-in evidence records results.
