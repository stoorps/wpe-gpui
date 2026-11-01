//! Async policy compilation is intentionally exercised in a fresh process.
use super::*;

struct PolicyChecks {
    view: Entity<WebView>,
    failed: bool,
    expect_failure: bool,
    ticks: usize,
}
impl PolicyChecks {
    fn tick(&mut self, cx: &mut Context<Self>) {
        self.ticks += 1;
        let status = self.view.read(cx).status();
        if self.expect_failure && self.failed {
            NativeChecks::check(
                status.frames == 0,
                "policy compile failure displays no WebKit frame",
            );
            NativeChecks::check(
                !status.loading,
                "policy compile failure ends queued loading",
            );
            NativeChecks::check(
                !status.error.is_empty(),
                "policy compile failure reports diagnostic",
            );
            let id = self.view.read(cx).state().document_id;
            NativeChecks::check(
                self.view.read(cx).load_html("<p>rejected</p>").is_err(),
                "failed policy refuses later document transaction",
            );
            NativeChecks::check(
                self.view.read(cx).state().document_id == id,
                "policy rejection preserves document identity",
            );
        } else if !self.expect_failure
            && status.title == "Latest queued policy document"
            && !status.loading
        {
            NativeChecks::check(
                !self.failed && status.error.is_empty(),
                "async policy compilation succeeds after pending view closes",
            );
            NativeChecks::check(
                self.view
                    .read(cx)
                    .sample_pixel(16, 16)
                    .is_ok_and(|p| p[..3] == [20, 40, 220]),
                "only latest queued restricted document is rendered",
            );
        } else {
            if self.ticks > 40 {
                NativeChecks::check(false, "async policy completion timeout");
                cx.quit();
            }
            return;
        }
        NativeChecks::check(
            gpui_wpe::live_metrics(cx).views == 1,
            "closed pending policy view remains released",
        );
        let metrics = gpui_wpe::live_metrics(cx);
        NativeChecks::check(
            metrics.policy_jobs == 1 && metrics.policy_waiters == 0,
            "completed policy leaves one bounded cache and no waiting views",
        );
        COMPLETED.store(true, Ordering::Relaxed);
        cx.quit();
    }
}
impl Render for PolicyChecks {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(self.view.clone())
    }
}
pub(super) fn run() {
    let expect_failure = std::env::var("GPUI_WPE_POLICY_TEST").is_ok_and(|v| v == "failure");
    gpui::platform::linux(gpui::WindowingModes::WAYLAND).run(move |cx: &mut App| {
        cx.open_window(WindowOptions::default(), move |window, cx| {
            let builder = || WebViewBuilder::new()
                .with_settings(settings(Theme::Auto))
                .with_resource_policy(ResourcePolicy::ProvidedOnly)
                .with_html("<title>Superseded queued policy document</title>");
            let mut closed = WebView::build(builder(), window, cx).expect("first pending policy view");
            NativeChecks::check(closed.status().loading && closed.status().frames == 0, "first restricted load waits for native policy compilation");
            closed.close(cx).expect("close before async policy completion");
            let metrics = gpui_wpe::live_metrics(cx);
            NativeChecks::check(metrics.views == 0 && metrics.policy_waiters == 0 && metrics.policy_jobs == 1,
                "closing a pending view removes its policy waiter");
            let view = WebView::build(builder(), window, cx).expect("second pending policy view");
            view.load_html("<!doctype html><title>Latest queued policy document</title><style>html{background:rgb(20,40,220)}</style>").expect("replace queued document");
            NativeChecks::check(view.status().loading && view.status().frames == 0, "replacement remains queued without unfiltered rendering");
            let metrics = gpui_wpe::live_metrics(cx);
            NativeChecks::check(metrics.policy_jobs == 1 && metrics.policy_waiters == 1,
                "pending replacement shares the bounded engine policy job");
            let view = cx.new(|_| view);
            cx.new(|cx: &mut Context<PolicyChecks>| {
                cx.subscribe(&view, |this, _, event, _| {
                    if matches!(event, WebViewEvent::Failure(failure) if failure.kind == FailureKind::Load) {
                        this.failed = true;
                    }
                }).detach();
                cx.spawn(async move |this, cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_millis(200)).await;
                        if this.update(cx, |state, cx| state.tick(cx)).is_err() { break; }
                    }
                }).detach();
                PolicyChecks { view, failed: false, expect_failure, ticks: 0 }
            })
        }).expect("policy test window");
        cx.activate(true);
    });
    if FAILED.load(Ordering::Relaxed) || !COMPLETED.load(Ordering::Relaxed) {
        std::process::exit(1);
    }
}
