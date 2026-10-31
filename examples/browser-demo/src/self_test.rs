//! Deterministic checks through GPUI's real event dispatcher and the WPE engine.
//! Run with --self-test. These complement, rather than replace, visual checks.
use super::*;
pub static FAILED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
#[derive(Default)]
pub struct SelfTest {
    stage: usize,
    after: Option<Instant>,
    failures: Vec<String>,
    resize_deadline: Option<Instant>,
}
impl SelfTest {
    fn check(&mut self, ok: bool, label: &str) {
        eprintln!("SELF-TEST {}: {label}", if ok { "PASS" } else { "FAIL" });
        if !ok {
            self.failures.push(label.into());
        }
    }
    fn next(&mut self) {
        self.stage += 1;
        self.after = Some(Instant::now() + Duration::from_millis(600));
    }
    pub fn tick(&mut self, b: &mut Browser, w: &mut Window, cx: &mut Context<Browser>) {
        if b.started.elapsed() > Duration::from_secs(45) {
            self.check(false, &format!("timeout at stage {}", self.stage));
            FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
            cx.quit();
            return;
        }
        if self.after.is_some_and(|time| Instant::now() < time) {
            return;
        }
        if b.tabs.is_empty() {
            self.check(false, &format!("cannot create WPE tab: {:?}", b.error));
            FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
            cx.quit();
            return;
        }
        let view = b.tabs[b.active].view.clone();
        match self.stage {
            0 => {
                if b.tabs[0].status.loading
                    || b.tabs[0].status.frames == 0
                    || b.tabs[0].status.title.is_empty()
                {
                    return;
                }
                self.check(
                    b.tabs[0].status.title == "Input interoperability lab",
                    "WPE fixture rendered",
                );
                view.read(cx).evaluate("JSON.stringify((()=>{let r=document.querySelector('input').getBoundingClientRect();return [r.x+20,r.y+20]})())");
                self.next();
            }
            1 => {
                let Some(result) = view.read(cx).take_evaluation() else {
                    return;
                };
                let p: Vec<f32> = serde_json::from_str(&result).expect("input coordinates");
                let origin = view.read(cx).viewport_bounds().origin;
                let position = origin + point(px(p[0]), px(p[1]));
                w.defer(cx, move |w, cx| {
                    w.dispatch_event(
                        PlatformInput::MouseMove(MouseMoveEvent {
                            position,
                            ..Default::default()
                        }),
                        cx,
                    );
                    w.dispatch_event(
                        PlatformInput::MouseDown(MouseDownEvent {
                            position,
                            button: MouseButton::Left,
                            click_count: 1,
                            ..Default::default()
                        }),
                        cx,
                    );
                    w.dispatch_event(
                        PlatformInput::MouseUp(MouseUpEvent {
                            position,
                            button: MouseButton::Left,
                            click_count: 1,
                            ..Default::default()
                        }),
                        cx,
                    );
                });
                self.next();
            }
            2 => {
                w.defer(cx, |w, cx| {
                    for c in ["i", "n", "t", "e", "r", "o", "p"] {
                        w.dispatch_keystroke(Keystroke::parse(c).unwrap(), cx);
                    }
                });
                self.next();
            }
            3 => {
                view.read(cx).paste_text(" clipboard");
                view.read(cx)
                    .evaluate("document.querySelector('input').value");
                self.next();
            }
            4 => {
                let Some(result) = view.read(cx).take_evaluation() else {
                    return;
                };
                self.check(
                    result == "interop clipboard",
                    &format!("typing via GPUI dispatcher and native paste ({result:?})"),
                );
                w.defer(cx, |w, cx| {
                    w.dispatch_keystroke(Keystroke::parse("ctrl-a").unwrap(), cx);
                    w.dispatch_keystroke(Keystroke::parse("ctrl-c").unwrap(), cx);
                });
                self.next();
            }
            5 => {
                let text = view.read(cx).last_copied_text().map(str::to_owned);
                self.check(
                    text.as_deref() == Some("interop clipboard"),
                    "web selection/copy reaches host clipboard bridge",
                );
                b.set_overlay(Overlay::Modal, w, cx);
                self.next();
            }
            6 => {
                w.defer(cx, |w, cx| {
                    w.dispatch_keystroke(Keystroke::parse("x").unwrap(), cx);
                });
                self.next();
            }
            7 => {
                view.read(cx)
                    .evaluate("document.querySelector('input').value");
                self.next();
            }
            8 => {
                let Some(result) = view.read(cx).take_evaluation() else {
                    return;
                };
                self.check(
                    result == "interop clipboard",
                    "modal typing does not leak into WPE",
                );
                self.check(
                    b.modal_input.read(cx).text().starts_with('x'),
                    "GPUI modal field receives typing",
                );
                w.resize(size(px(950.), px(640.)));
                self.resize_deadline = Some(Instant::now() + Duration::from_secs(4));
                b.set_overlay(Overlay::Menu(2), w, cx);
                self.next();
            }
            9 => {
                view.read(cx).evaluate("String(innerWidth)");
                self.next();
            }
            10 => {
                let Some(result) = view.read(cx).take_evaluation() else {
                    return;
                };
                let expected = f32::from(view.read(cx).viewport_bounds().size.width).round() as i32;
                let matches = result
                    .parse::<i32>()
                    .ok()
                    .is_some_and(|n| (n - expected).abs() <= 1);
                if !matches && self.resize_deadline.is_some_and(|d| Instant::now() < d) {
                    self.stage = 9;
                    self.after = Some(Instant::now() + Duration::from_millis(100));
                    return;
                }
                self.check(
                    matches,
                    &format!("WPE viewport tracks GPUI resize ({result} vs {expected})"),
                );
                b.set_overlay(Overlay::None, w, cx);
                b.add_tab("demo://second/", w, cx);
                self.next();
            }
            11 => {
                if b.tabs[b.active].status.loading {
                    return;
                }
                self.check(
                    b.tabs.len() == 2 && b.tabs[1].status.title == "Second page",
                    "independent second tab loads",
                );
                b.activate(0, w, cx);
                self.next();
            }
            12 => {
                view.read(cx)
                    .evaluate("document.querySelector('input').value");
                self.next();
            }
            13 => {
                let Some(result) = view.read(cx).take_evaluation() else {
                    return;
                };
                self.check(
                    result == "interop clipboard",
                    "tab switch preserves edited document",
                );
                b.activate(1, w, cx);
                b.command(Command::CloseTab, w, cx);
                self.check(b.tabs.len() == 1, "closing tab removes its WPE surface");
                view.read(cx)
                    .evaluate("location.href='demo://second/';'navigating'");
                self.next();
            }
            14 => {
                if b.tabs[0].status.loading || b.tabs[0].status.title != "Second page" {
                    return;
                }
                self.check(b.tabs[0].status.can_back, "navigation enables Back");
                b.command(Command::Back, w, cx);
                self.next();
            }
            15 => {
                if b.tabs[0].status.loading
                    || b.tabs[0].status.title != "Input interoperability lab"
                {
                    return;
                }
                self.check(
                    b.tabs[0].status.can_forward,
                    "Back restores page and enables Forward",
                );
                b.command(Command::Forward, w, cx);
                self.next();
            }
            16 => {
                if b.tabs[0].status.loading || b.tabs[0].status.title != "Second page" {
                    return;
                }
                self.check(true, "Forward restores second page");
                b.command(Command::EmailFixture, w, cx);
                self.next();
            }
            17 => {
                if b.tabs[0].status.loading {
                    return;
                }
                self.check(
                    b.tabs[0].status.uri == "message://body/" && b.tabs[0].status.frames > 0,
                    "sanitized offline message renders",
                );
                self.check(
                    b.tabs[0].status.error.is_empty(),
                    "WPE reports no load/render failure",
                );
                eprintln!("SELF-TEST completed: {} failures", self.failures.len());
                if !self.failures.is_empty() {
                    FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                cx.quit();
                self.next();
            }
            _ => {}
        }
    }
}
