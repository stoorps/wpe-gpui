# Embedding a WebView

Use `gpui-wpe` from a native Wayland GPUI Fast application. It manages WPE servicing
and native teardown; the host supplies layout, application state, and UI decisions.
See [getting started](getting-started.md) for native dependencies and
[compatibility](compatibility.md) for the required GPUI type identities.

## Construct and render

For a local checkout, the consuming application's dependencies include:

```toml
[dependencies]
gpui = { package = "gpui-kit", version = "=0.8.1", default-features = false, features = ["gpui-fast"] }
gpui-wpe = { path = "../wpe-gpui-fast/crates/gpui-wpe" }
```

Create the native view while the GPUI window is available, then put it in an entity:

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

Store `Entity<WebView>` in your host and return `div().size_full().child(view.clone())`
from its `Render` implementation. The view implements `Render` and `Focusable`.
The [standalone example](../examples/embedding/src/main.rs) includes window creation
and a complete host. It requires neither the browser demo nor the email consumer.

The builder accepts `with_url`, `with_html`, or `with_document`; the most recent
content setter replaces the previous choice. Settings and resource policy are fixed
at construction. Recreate the view when changing those contracts.

## State, events, and commands

Read `state()` for URL, title, loading/history availability, document identity,
closed state, and errors. Subscribe to `WebViewEvent` through GPUI's normal entity
event mechanism and retain the returned subscription in the host. Important events
are `StateChanged`, `LoadChanged`, `NavigationRequested`, `FindChanged`,
`ContextMenuRequested`, `SelectionChanged`, `SelectionCopied`, `Failure`, `Error`,
and `Closed`.

Use `try_load_url()` for immediate validation errors. The upstream-shaped
`load_url()` convenience method returns `()` and queues errors as events. Accepted
loads can fail asynchronously. `back()`, `forward()`, `reload()`, and `stop_loading()`
operate through the native engine, including when page scripting is disabled.

Keep application UI above the webview in GPUI. For a modal, use host focus handling
and `block_input(true)` while it owns interaction; unblock and restore focus when
it closes. Hiding a view cancels its active input state. The browser demo is a
reference for tabs, menus, modal focus, find, and context actions.

## Supplied documents and resources

`Document::html()` creates a new identity. Its base URL is generated as
`gpui-document://{id}/body`; the current API has no arbitrary base-URL setter.
Use `resource::resource_uri()` for explicit `gpui-resource://{id}/{key}` URLs.
Keys are opaque ASCII identifiers, not filesystem paths.

```rust
use gpui_wpe::{Document, resource::resource_uri};

fn document_with_styles() -> gpui_wpe::Result<Document> {
    let document = Document::html("");
    let css = resource_uri(document.id(), "style.css")?;
    let document = document.with_html(format!(
        "<link rel=\"stylesheet\" href=\"{css}\"><h1>Hello</h1>"
    ));
    document.with_resource("style.css", b"h1 { color: navy; }".to_vec(), "text/css")
}
```

Pass the prepared document to `with_document()` or `load_document()`. HTML loading
does not sanitize content or change settings. Modify the prepared body only before
its identity is first loaded. Use a fresh `Document` when changing content; loading
different HTML under an already-loaded identity is rejected. Prepared documents
may be loaded in a different order from allocation.

For asynchronous resources, install `with_resource_provider(|request, responder| ...)`
on the builder. The callback runs on the UI thread: move the request and responder
to your own bounded worker rather than doing blocking work there. Respond with
`ResourceResponse`, or call `fail()`. Dropping the responder fails the request.
Check `is_cancelled()` around expensive work. The library bounds pending requests,
queued bytes, and deadlines; replacement/close invalidates old responses.
`provide_resource(id, key, response)` also rejects an obsolete current identity.

These resources are document-scoped complete byte responses, not a general HTTP
server, filesystem protocol, streaming API, or range-request interface. See
[security](security.md) for limits and restricted-network configuration.

## Geometry and visibility

Normal GPUI layout sets geometry during prepaint. `set_geometry(document, clip)`
uses logical window coordinates and keeps the document viewport separate from its
visible clip. Clip-only changes must not reflow content. `set_bounds(Rect)` uses the
same rectangle for both. The core validates dimensions and physical pixel limits;
it propagates the window's output scale independently of page zoom.

Use `set_visible()` or `show()` / `hide()` for application visibility. User zoom is
controlled by `set_zoom()` / `zoom()`. Query `capabilities()` before offering optional
operations; selected-text availability can change with the shared engine session.

## Lifetime and failures

`WebView` and `WebViewHandle` are UI-thread-local. A cloned handle is weak: it does
not keep the native surface alive after its view/window closes. Handle commands
return `Error::Closed` once invalidated. `ResourceResponder` is the worker-safe
completion capability; it does not expose native pointers.

The library registers app-quit and window-close hooks. It stops the native producer
and releases composition ownership before the host surface/display is destroyed,
even if another entity retains the view. Explicit `close(cx)` is available and
idempotent, but is not required for ordinary safety. Do not copy the old prototype's
manual pump/shutdown sequence into a new host.

Use typed `Failure` events to present host-owned recovery UI. Reload may recover a
web-process/load failure while retaining restrictions; a renderer failure can
require a newly configured view. A shared clipboard protocol failure requires an
application restart. Catastrophic GPU/driver hangs do not have a guaranteed graceful
recovery path. See [support](support.md) and [architecture](architecture.md).
