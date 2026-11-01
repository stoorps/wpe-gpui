//! Native lifecycle check: closing one OS window stops a retained WebView.
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Window, WindowBounds, WindowHandle, WindowOptions, div, px, size,
};
use gpui_wpe::{
    Document, Error, WebView, WebViewBuilder, WebViewEvent, WebViewHandle,
    policy::ResourcePolicy,
    settings::{JavaScript, WebViewSettings},
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

static FAILED: AtomicBool = AtomicBool::new(false);
static COMPLETED: AtomicBool = AtomicBool::new(false);
struct WindowChecks {
    first: Entity<WebView>,
    retained: Option<Entity<WebView>>,
    retained_handle: Option<WebViewHandle>,
    second_window: Option<WindowHandle<WebView>>,
    closed_events: usize,
    stage: usize,
    started: Instant,
    stage_started: Instant,
    first_frames: u64,
}
fn check(ok: bool, label: &str) {
    eprintln!(
        "WINDOW_LIFECYCLE {} {label}",
        if ok { "PASS" } else { "FAIL" }
    );
    if !ok {
        FAILED.store(true, Ordering::Relaxed);
    }
}
fn builder(title: &str) -> WebViewBuilder {
    WebViewBuilder::new().with_settings(WebViewSettings{javascript:JavaScript::Disabled,..Default::default()})
        .with_resource_policy(ResourcePolicy::ProvidedOnly)
        .with_html(format!("<!doctype html><title>{title}</title><h1>{title}</h1><p>Original local window lifecycle fixture.</p>"))
}
impl WindowChecks {
    fn attach(
        &mut self,
        second: Entity<WebView>,
        handle: WindowHandle<WebView>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.retained_handle = Some(second.read(cx).handle());
        cx.subscribe_in(&second, window, |this, _, event: &WebViewEvent, _, cx| {
            if matches!(event, WebViewEvent::Closed) {
                this.closed_events += 1;
                cx.notify();
            }
        })
        .detach();
        self.retained = Some(second);
        self.second_window = Some(handle);
    }
    fn next(&mut self) {
        self.stage += 1;
        self.stage_started = Instant::now();
    }
    fn tick(&mut self, cx: &mut Context<Self>) {
        if self.started.elapsed() > Duration::from_secs(45) {
            check(false, "completion timeout");
            cx.quit();
            return;
        }
        match self.stage {
            0 => {
                let first = self.first.read(cx).status();
                let Some(second) = &self.retained else {
                    return;
                };
                let status = second.read(cx).status();
                if first.frames == 0 || status.frames == 0 || first.loading || status.loading {
                    return;
                }
                check(
                    gpui_wpe::live_metrics(cx).views == 2,
                    "two OS windows have independent live native views",
                );
                check(
                    !second.read(cx).state().closed,
                    "retained second view starts open",
                );
                self.first_frames = first.frames;
                self.second_window
                    .take()
                    .expect("second window handle")
                    .update(cx, |_, window, _| window.remove_window())
                    .expect("close second OS window");
                self.next();
            }
            1 => {
                let Some(second) = &self.retained else {
                    return;
                };
                let closed = second.read(cx).state().closed;
                let handle_closed = matches!(
                    self.retained_handle
                        .as_ref()
                        .expect("retained handle")
                        .set_zoom(1.1),
                    Err(Error::Closed)
                );
                if (!closed
                    || !handle_closed
                    || self.closed_events == 0
                    || gpui_wpe::live_metrics(cx).views != 1)
                    && self.stage_started.elapsed() < Duration::from_secs(5)
                {
                    return;
                }
                check(
                    closed,
                    "state reports closed while Entity is retained by another window",
                );
                check(
                    handle_closed,
                    "retained weak handle returns Closed without native access",
                );
                check(
                    self.closed_events == 1,
                    "Closed event delivered exactly once to surviving window",
                );
                check(
                    gpui_wpe::live_metrics(cx).views == 1,
                    "window removal releases second native producer before Entity drop",
                );
                self.first.read(cx).load_document(Document::html("<!doctype html><title>Surviving window reloaded</title><h1>Still alive</h1>")).expect("load surviving view");
                self.next();
            }
            2 => {
                let status = self.first.read(cx).status();
                if status.title != "Surviving window reloaded"
                    || status.loading
                    || status.frames <= self.first_frames
                {
                    return;
                }
                check(
                    status.error.is_empty() && !self.first.read(cx).state().closed,
                    "first OS window continues loading and rendering",
                );
                self.retained.take();
                self.next();
            }
            3 if self.stage_started.elapsed() > Duration::from_millis(300) => {
                check(
                    gpui_wpe::live_metrics(cx).views == 1,
                    "dropping retained closed Entity does not stop surviving view",
                );
                check(
                    self.closed_events == 1,
                    "Entity release emits no duplicate Closed event",
                );
                COMPLETED.store(true, Ordering::Relaxed);
                cx.quit();
            }
            _ => {}
        }
    }
}
impl Render for WindowChecks {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.first.clone())
    }
}
fn options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(720.), px(540.)),
            cx,
        ))),
        ..Default::default()
    }
}
fn main() {
    gpui::platform::linux(gpui::WindowingModes::WAYLAND).run(|cx: &mut App| {
        let first = cx
            .open_window(options(cx), |window, cx| {
                window.on_window_should_close(cx, |_, cx| {
                    cx.quit();
                    true
                });
                let view = WebView::build(builder("Surviving window"), window, cx)
                    .expect("create first view");
                let first = cx.new(|_| view);
                cx.new(|cx: &mut Context<WindowChecks>| {
                    cx.spawn(async move |this, cx| {
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(20))
                                .await;
                            if this.update(cx, |this, cx| this.tick(cx)).is_err() {
                                break;
                            }
                        }
                    })
                    .detach();
                    WindowChecks {
                        first,
                        retained: None,
                        retained_handle: None,
                        second_window: None,
                        closed_events: 0,
                        stage: 0,
                        started: Instant::now(),
                        stage_started: Instant::now(),
                        first_frames: 0,
                    }
                })
            })
            .expect("open first window");
        let second = cx
            .open_window(options(cx), |window, cx| {
                let view = WebView::build(builder("Retained second window"), window, cx)
                    .expect("create second view");
                cx.new(|_| view)
            })
            .expect("open second window");
        let retained = second
            .update(cx, |_, _, cx| cx.entity())
            .expect("retain second view outside its window");
        first
            .update(cx, |host, window, cx| {
                host.attach(retained, second, window, cx)
            })
            .expect("attach retained view to surviving host");
        cx.activate(true);
    });
    if FAILED.load(Ordering::Relaxed) || !COMPLETED.load(Ordering::Relaxed) {
        std::process::exit(1);
    }
}
