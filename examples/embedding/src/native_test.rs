//! Native Wayland regression checks for the scripting-disabled embedding API.
#[cfg(feature = "native-failure-injection")]
use gpui::AnyWindowHandle;
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Window, WindowBounds, WindowOptions, div, px, size,
};
use gpui_wpe::{
    FailureKind, LoadPhase, WebView, WebViewBuilder, WebViewEvent,
    policy::ResourcePolicy,
    settings::{JavaScript, Theme, WebViewSettings},
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

static FAILED: AtomicBool = AtomicBool::new(false);
static COMPLETED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "native-failure-injection")]
mod native_policy_test;
const SELECTED: &str = "selection-native-token — Καλημέρα 日本語";
const HTML: &str = r#"<!doctype html>
<title>Native feature checks</title>
<style>html{background:rgb(240,20,20)}body{margin:0;padding-top:50px}</style>
<p>selection-native-token — Καλημέρα 日本語</p>"#;

struct NativeChecks {
    first: Entity<WebView>,
    second: Entity<WebView>,
    stage: usize,
    ticks: usize,
    selected: Option<String>,
    copied: bool,
    crashed: bool,
    frames_before_crash: u64,
    phases: Vec<LoadPhase>,
    server_url: String,
    second_document: Option<gpui_wpe::DocumentId>,
    second_crashed: bool,
    #[cfg(feature = "native-failure-injection")]
    window: AnyWindowHandle,
}
impl NativeChecks {
    fn check(ok: bool, label: &str) {
        eprintln!(
            "NATIVE_FEATURE {} {label}",
            if ok { "PASS" } else { "FAIL" }
        );
        if !ok {
            FAILED.store(true, Ordering::Relaxed);
        }
    }
    fn tick(&mut self, cx: &mut Context<Self>) {
        self.ticks += 1;
        if self.ticks > 90 {
            Self::check(false, "completion timeout");
            cx.quit();
            return;
        }
        match self.stage {
            0 if self.ticks > 12 => {
                let first = self.first.read(cx).sample_pixel(16, 16).unwrap_or([0; 4]);
                let second = self.second.read(cx).sample_pixel(16, 16).unwrap_or([0; 4]);
                Self::check(
                    first[..3] == [240, 20, 20] && second[..3] == [20, 40, 220],
                    "two independent views render expected pixels",
                );
                Self::check(
                    [
                        LoadPhase::Started,
                        LoadPhase::Committed,
                        LoadPhase::Finished,
                    ]
                    .iter()
                    .all(|phase| self.phases.contains(phase)),
                    "queued start, commit, and finish events delivered",
                );
                Self::check(
                    self.first.read(cx).capabilities().selected_text,
                    "selection query available in exclusively scripting-disabled engine",
                );
                #[cfg(feature = "native-failure-injection")]
                if std::env::var_os("GPUI_WPE_FAIL_STAGE").is_some() {
                    let baseline = gpui_wpe::live_metrics(cx);
                    for _ in 0..8 {
                        let result = self
                            .window
                            .update(cx, |_, window, cx| {
                                WebView::build(
                                    WebViewBuilder::new().with_settings(settings(Theme::Auto)),
                                    window,
                                    cx,
                                )
                            })
                            .expect("live fault test window");
                        Self::check(
                            result.as_ref().err().is_some_and(|error| {
                                error
                                    .to_string()
                                    .contains("Injected native constructor failure")
                            }),
                            "injected partial constructor returns expected diagnostic error",
                        );
                        Self::check(
                            gpui_wpe::live_metrics(cx) == baseline,
                            "failed constructor releases native views/imports/sources/resources",
                        );
                    }
                    Self::check(
                        self.first
                            .read(cx)
                            .sample_pixel(16, 16)
                            .is_ok_and(|p| p[..3] == [240, 20, 20])
                            && self
                                .second
                                .read(cx)
                                .sample_pixel(16, 16)
                                .is_ok_and(|p| p[..3] == [20, 40, 220]),
                        "existing renderer contexts survive repeated constructor failures",
                    );
                    COMPLETED.store(true, Ordering::Relaxed);
                    cx.quit();
                    return;
                }
                self.second_document = self.second.read(cx).state().document_id;
                self.first.read(cx).select_all().expect("select text");
                self.stage = 1;
                self.ticks = 0;
            }
            1 if self.ticks > 3 => {
                Self::check(
                    self.first
                        .read(cx)
                        .editing_state()
                        .is_ok_and(|state| state.can_copy),
                    "native editor reports a copyable selection",
                );
                self.first
                    .read(cx)
                    .selected_text()
                    .expect("request selected text");
                self.stage = 2;
                self.ticks = 0;
            }
            2 if self.selected.is_some() => {
                Self::check(
                    self.selected
                        .take()
                        .is_some_and(|text| text.contains(SELECTED)),
                    "selected UTF-8 text queried without JavaScript",
                );
                // The core writes to the host clipboard only for SelectionCopied.
                // No cross-application clipboard/Wayland serial is needed here.
                Self::check(!self.copied, "query emits no host clipboard write");
                self.first.read(cx).selected_text().expect("second query");
                self.first
                    .update(cx, |view, cx| view.copy(cx))
                    .expect("concurrent user copy");
                self.stage = 3;
                self.ticks = 0;
            }
            3 if self.selected.is_some() && self.copied => {
                Self::check(
                    self.selected
                        .take()
                        .is_some_and(|text| text.contains(SELECTED)),
                    "second query completes before deferred user Copy",
                );
                Self::check(self.copied, "concurrent user Copy is delivered");
                self.frames_before_crash = self.first.read(cx).status().frames;
                self.first
                    .read(cx)
                    .diagnostic_terminate_process()
                    .expect("terminate test process");
                self.stage = 4;
                self.ticks = 0;
            }
            4 if self.crashed => {
                Self::check(true, "typed process termination delivered");
                self.first
                    .read(cx)
                    .reload()
                    .expect("reload after process termination");
                self.stage = 5;
                self.ticks = 0;
            }
            5 if self.ticks > 8 => {
                let status = self.first.read(cx).status();
                Self::check(
                    status.frames > self.frames_before_crash
                        && !status.loading
                        && status.error.is_empty(),
                    "reload recovers terminated WebKit process",
                );
                Self::check(
                    self.first
                        .update(cx, |view, _| view.try_load_url(&self.server_url))
                        .is_err(),
                    "restricted view still denies network load after process recovery",
                );
                self.second
                    .update(cx, |view, _| view.try_load_url(&self.server_url))
                    .expect("load local HTTP fixture");
                self.stage = 6;
                self.ticks = 0;
            }
            6 if self.second.read(cx).status().title == "HTTP transition fixture"
                && !self.second.read(cx).status().loading =>
            {
                Self::check(
                    self.second.read(cx).state().document_id.is_none(),
                    "HTTP commit revokes active application document identity",
                );
                // Force WebKit's old document out of the process/BF cache so
                // Back must request its custom-scheme body before COMMITTED.
                self.second
                    .read(cx)
                    .diagnostic_terminate_process()
                    .expect("terminate HTTP process");
                self.stage = 7;
                self.ticks = 0;
            }
            7 if self.second_crashed => {
                self.second.read(cx).reload().expect("recover HTTP process");
                self.stage = 8;
                self.ticks = 0;
            }
            8 if self.second.read(cx).status().title == "HTTP transition fixture"
                && !self.second.read(cx).status().loading
                && self.second.read(cx).status().error.is_empty() =>
            {
                self.second
                    .update(cx, |view, _| view.back())
                    .expect("back to application document");
                self.stage = 9;
                self.ticks = 0;
            }
            9 if self.second.read(cx).status().title == "Native history checks"
                && !self.second.read(cx).status().loading =>
            {
                Self::check(
                    self.second.read(cx).state().document_id == self.second_document,
                    "Document to HTTP to Back restores original identity after process restart",
                );
                Self::check(
                    self.second
                        .read(cx)
                        .sample_pixel(16, 16)
                        .is_ok_and(|p| p[..3] == [20, 40, 220]),
                    "history document body re-fetch renders original independent content",
                );
                COMPLETED.store(true, Ordering::Relaxed);
                cx.quit();
            }
            _ => {}
        }
    }
}
impl Render for NativeChecks {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .child(div().w(px(450.)).h_full().child(self.first.clone()))
            .child(div().w(px(450.)).h_full().child(self.second.clone()))
    }
}
fn settings(theme: Theme) -> WebViewSettings {
    WebViewSettings {
        javascript: JavaScript::Disabled,
        theme,
        ..Default::default()
    }
}
fn http_fixture() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("local HTTP fixture listener");
    let address = listener.local_addr().expect("fixture address");
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let body = "<!doctype html><title>HTTP transition fixture</title><p>Original local HTTP fixture</p>";
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{address}/")
}
fn main() {
    #[cfg(feature = "native-failure-injection")]
    if std::env::var_os("GPUI_WPE_POLICY_TEST").is_some() {
        native_policy_test::run();
        return;
    }
    let server_url = http_fixture();
    gpui::platform::linux(gpui::WindowingModes::WAYLAND).run(move|cx: &mut App| {
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(900.), px(600.)),
                    cx,
                ))),
                ..Default::default()
            },
            move|window, cx| {
                for theme in [Theme::Light, Theme::Dark] {
                    unsupported_theme(theme, window, cx);
                }
                let first = WebView::build(
                    WebViewBuilder::new()
                        .with_settings(settings(Theme::Auto))
                        .with_resource_policy(ResourcePolicy::ProvidedOnly)
                        .with_html(HTML),
                    window,
                    cx,
                )
                .expect("first scripting-disabled view");
                let second = WebView::build(
                    WebViewBuilder::new()
                        .with_settings(settings(Theme::Auto))
                        .with_html(HTML.replace("rgb(240,20,20)","rgb(20,40,220)").replace("Native feature checks","Native history checks")),
                    window,
                    cx,
                )
                .expect("second scripting-disabled view");
                let first = cx.new(|_| first);
                let second = cx.new(|_| second);
                #[cfg(feature="native-failure-injection")]
                let handle=window.window_handle();
                cx.new(|cx: &mut Context<NativeChecks>| {
                    cx.subscribe(&first, |this, _, event, cx| {
                        match event {
                            WebViewEvent::SelectionChanged(state) => {
                                this.selected = Some(state.text.clone())
                            }
                            WebViewEvent::SelectionCopied(_) => this.copied = true,
                            WebViewEvent::LoadChanged(event) => this.phases.push(event.phase),
                            WebViewEvent::Failure(failure)
                                if failure.kind == FailureKind::ProcessTerminated =>
                            {
                                this.crashed = true
                            }
                            WebViewEvent::Error(error) => {
                                eprintln!("NATIVE_FEATURE event {error:?}")
                            }
                            _ => {}
                        }
                        cx.notify();
                    })
                    .detach();
                    cx.subscribe(&second,|this,_,event,_|{
                        if matches!(event,WebViewEvent::Failure(failure)if failure.kind==FailureKind::ProcessTerminated){this.second_crashed=true;}
                    }).detach();
                    cx.spawn(async move |this, cx| {
                        loop {
                            cx.background_executor()
                                .timer(Duration::from_millis(200))
                                .await;
                            if this.update(cx, |state, cx| state.tick(cx)).is_err() {
                                break;
                            }
                        }
                    })
                    .detach();
                    NativeChecks {
                        first,
                        second,
                        stage: 0,
                        ticks: 0,
                        selected: None,
                        copied: false,
                        crashed: false,
                        frames_before_crash: 0,
                        phases: vec![],
                        server_url,second_document:None,second_crashed:false,
                        #[cfg(feature="native-failure-injection")]
                        window:handle,
                    }
                })
            },
        )
        .expect("native feature test window");
        cx.activate(true);
    });
    if FAILED.load(Ordering::Relaxed) || !COMPLETED.load(Ordering::Relaxed) {
        std::process::exit(1);
    }
}
fn unsupported_theme(theme: Theme, window: &mut Window, cx: &mut App) {
    NativeChecks::check(
        matches!(
            WebView::build(
                WebViewBuilder::new().with_settings(settings(theme)),
                window,
                cx
            ),
            Err(gpui_wpe::Error::Unsupported(_))
        ),
        "per-view color scheme fails explicitly as Unsupported",
    );
}
