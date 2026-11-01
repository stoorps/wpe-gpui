# GPUI and gpui-webview compatibility

The workspace aliases `gpui` to **gpui-kit 0.8.1**, with default components/assets
disabled and the `gpui-fast` feature selected. Its public core resolves to
**gpui-fast 0.1.12**; the platform alias selects **gpui-fast-platform 0.1.12**.
Use these same Rust package identities in a host. Matching names such as `App`
and `Window` from different GPUI packages/revisions do not establish type identity.

## Construction and intentional differences

| Surface | Current WPE behavior |
| --- | --- |
| Construction | `WebView::build(builder, window, cx)` returns crate `Result<Self>`. The builder is `gpui_wpe::WebViewBuilder`, not Wry's builder. |
| GPUI integration | `Entity<WebView>`, `Render`, `Focusable`, GPUI layout geometry and event subscriptions. |
| URL loading | `load_url(&mut self, &str)` queues validation errors as events; `try_load_url` returns them directly. |
| Navigation/visibility/zoom | Familiar `back`, `forward`, `reload`, `set_visible`, `show`, `hide`, `set_zoom`, and `zoom` vocabulary, with crate-specific errors. |
| Geometry | `set_geometry` uses GPUI document/clip bounds. `geometry::Rect` is an owned GPUI-based type, not `wry::Rect`. |
| Theme | An owned `Theme` enum; only `Auto` is supported by the reference WPE API. |
| Handle | Weak UI-thread-local `WebViewHandle`; commands fail after close rather than extending native surface lifetime. |
| Raw Wry integration | No `raw()`/Wry `Deref`, raw-Wry adoption constructor, or interchangeable raw handles. |
| Resource/policy APIs | WPE-specific managed extensions; no claim of identical security semantics across engines. |

The compatibility baseline is `gpui-webview` **0.8.1** and **lb-wry 0.53.3**.
Its source was compared at GPUI Kit revision
[`944803984cf0ba3111861d6e1aa9d6265c109ff9`](https://github.com/longbridge/gpui-kit/tree/944803984cf0ba3111861d6e1aa9d6265c109ff9/crates/webview).
Re-audit selected versions when updating; this is not a promise of parity with an
unpinned upstream branch or every Wry feature.

## Verify type identity

```sh
cargo check -p embedding-example --locked
cargo tree -p embedding-example --invert gpui-fast
cargo check -p embedding-example --features upstream-compat --locked
```

The [embedding example](../examples/embedding/src/main.rs) assigns facade `App`
and `Window` references directly to the published Fast core types. The optional
feature also typechecks the actual upstream
`gpui_webview::WebView::build(wry::WebViewBuilder, window, cx)` call. It does not
launch a second engine or test Windows/macOS behavior.

On Linux, the optional proof needs GTK3/WebKitGTK development packages. Those Wry
dependencies are absent from the normal WPE consumer path. GPUI Kit itself still
includes GPUI Base and some `gpui-pre` dependencies internally; that does not mean
the Fast-selected public facade exposes those types. A duplicate-package report
alone is not a type-identity proof.

## Cross-platform application structure

An application can use WPE on Linux and upstream `gpui-webview` on Windows/macOS
through its own adapter. Share GPUI entity/layout conventions and expose the small
operation set the application needs. Translate builder options, errors, geometry,
events, and capability differences explicitly. Do not simply alias the two builders
and assume matching methods enforce the same restrictions.

Restricted-network behavior, resource delivery, clipboard queries, native theme,
and lifecycle handles require backend-specific review. No Windows/macOS execution
is established by the Linux compile proof. See [API structure](api.md),
[security](security.md), and [support](support.md).
