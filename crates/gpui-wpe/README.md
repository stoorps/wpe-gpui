# gpui-wpe

Experimental native Wayland WPE WebKit views for GPUI Fast, using GPUI Kit's Fast
facade. The crate provides a GPUI `WebView` entity, `WebViewBuilder`, document and
resource APIs, explicit settings/policy configuration, and library-owned lifecycle
and event-loop integration. Email sanitization, CID rules and remote-image consent
belong to the application-level `email-renderer` example, not this crate.

Requires Linux Wayland, WPE WebKit/WPEPlatform 2.54 or newer, EGL/GLES, Wayland,
xkbcommon and DRM development libraries, plus GPUI's platform dependencies. Actual
supported runtime/build and graphics configurations must be verified; this is not
a promise of compatibility with every GPU or version of WebKit. Native dependencies
are dynamically linked and retain their own licenses.

A host with an existing GPUI window can build and render an entity:

```rust,ignore
use gpui::{App, AppContext as _, Entity, Window};
use gpui_wpe::{WebView, WebViewBuilder};

fn create(window: &mut Window, cx: &mut App) -> gpui_wpe::Result<Entity<WebView>> {
    let view = WebView::build(
        WebViewBuilder::new().with_html("<h1>Hello from WPE</h1>"),
        window,
        cx,
    )?;
    Ok(cx.new(|_| view))
}
// In the host's Render implementation: div().child(view.clone()).
```

The host must select the same GPUI Kit/Fast dependency identities as the crate.
Current source examples, tested capabilities, known exclusions, security-update
setup and productionisation status are documented in the
[repository](https://github.com/stoorps/wpe-gpui-fast).
Do not treat successful HTML loading as automatic sanitization or email hardening.

Project source is Apache-2.0; see the packaged `LICENSE-APACHE`. The repository's
`NOTICE` and `THIRD_PARTY.md` preserve upstream attribution and identify dependency
licenses. Binary/native-runtime distributions need the applicable third-party
license, notice and source materials for the exact artifact; this crate's license
does not relicense WebKit or its dependencies.
