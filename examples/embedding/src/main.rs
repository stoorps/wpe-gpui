//! Minimal app-owned consumer: no demo controls, sanitizer, or manual native pump.
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Window, WindowBounds, WindowOptions, div, px, size,
};
use gpui_wpe::{WebView, WebViewBuilder};
static SMOKE_FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

struct Embedding {
    view: Entity<WebView>,
}
impl Render for Embedding {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.view.clone())
    }
}

// These bodies compile only when aliases resolve to the exact same Rust types.
#[allow(dead_code)]
fn app_identity(app: &mut App) -> &mut gpui_fast_core::App {
    app
}
#[allow(dead_code)]
fn window_identity(window: &mut Window) -> &mut gpui_fast_core::Window {
    window
}

// This function is typechecked but never launches GTK/X11 or a second engine.
// Enable upstream-compat to prove gpui-webview consumes these same App/Window types.
#[cfg(feature = "upstream-compat")]
#[allow(dead_code)]
fn upstream_constructor(window: &mut Window, cx: &mut App) -> wry::Result<gpui_webview::WebView> {
    gpui_webview::WebView::build(wry::WebViewBuilder::new(), window, cx)
}

fn main() {
    gpui::platform::linux(gpui::WindowingModes::WAYLAND).run(|cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(960.), px(720.)), cx);
        cx.open_window(
            WindowOptions { window_bounds: Some(WindowBounds::Windowed(bounds)), ..Default::default() },
            |window, cx| {
                window.on_window_should_close(cx, |_, cx| { cx.quit(); true });
                let view = WebView::build(WebViewBuilder::new().with_html("<!doctype html><meta charset=\"utf-8\"><title>Embedding example</title><h1>Standalone WPE consumer</h1><p>No demo controls, email profile, or manual native pump.</p>"), window, cx)
                    .expect("create native Wayland WebView");
                let entity=cx.new(|_| view);
                if std::env::var_os("EMBEDDING_SMOKE").is_some() {
                    let weak=entity.downgrade();
                    cx.spawn(async move |cx| {
                        cx.background_executor().timer(std::time::Duration::from_secs(12)).await;
                        let status=weak.update(cx,|view,_|view.status()).expect("embedding still alive");
                        eprintln!("EMBEDDING_SMOKE {status:?}");
                        let failed=status.frames==0 || status.loading || !status.error.is_empty();
                        SMOKE_FAILED.store(failed,std::sync::atomic::Ordering::Relaxed);
                        cx.update(|cx|cx.quit());
                    }).detach();
                }
                cx.new(|_| Embedding {view:entity})
            },
        ).expect("open embedding window");
        cx.activate(true);
    });
    if SMOKE_FAILED.load(std::sync::atomic::Ordering::Relaxed) {
        std::process::exit(1);
    }
}
