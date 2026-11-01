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
    documents: Option<(Document, Document)>,
    selection: std::rc::Rc<std::cell::RefCell<Option<String>>>,
    selection_supported: bool,
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
                view.read(cx).evaluate("alert('blocked');if(confirm('blocked')!==false||prompt('blocked')!==null)throw Error('unhandled dialog granted');JSON.stringify((()=>{let r=document.querySelector('input').getBoundingClientRect();return [r.x+20,r.y+20]})())").expect("trusted diagnostic evaluation");
                self.next();
            }
            1 => {
                let Some(result) = view.read(cx).take_evaluation() else {
                    return;
                };
                let p: Vec<f32> = match serde_json::from_str(&result) {
                    Ok(value) => value,
                    Err(_) => {
                        self.check(
                            false,
                            "blocked JS dialogs resume without granting confirmation/prompt",
                        );
                        FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
                        cx.quit();
                        return;
                    }
                };
                self.check(
                    true,
                    "blocked JS dialogs resume without granting confirmation/prompt",
                );
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
                view.read(cx)
                    .paste_text(" clipboard")
                    .expect("native paste");
                view.read(cx)
                    .evaluate("document.querySelector('input').value")
                    .expect("trusted diagnostic evaluation");
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
                    .evaluate("document.querySelector('input').value")
                    .expect("trusted diagnostic evaluation");
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
                view.read(cx)
                    .evaluate("String(innerWidth)")
                    .expect("trusted diagnostic evaluation");
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
                    .evaluate("document.querySelector('input').value")
                    .expect("trusted diagnostic evaluation");
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
                    .evaluate("location.href='demo://second/';'navigating'")
                    .expect("trusted diagnostic evaluation");
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
                if b.tabs[0].status.loading || b.tabs[0].status.frames == 0 {
                    return;
                }
                self.check(
                    b.tabs[0].status.uri.starts_with("gpui-document://")
                        && b.tabs[0].status.frames > 0
                        && b.tabs[0].email.is_some(),
                    "sanitized offline message renders",
                );
                self.check(
                    b.tabs[0].status.error.is_empty(),
                    "WPE reports no load/render failure",
                );
                self.check(
                    view.read(cx).evaluate("document.title").is_err(),
                    "email view refuses script evaluation",
                );
                self.check(
                    view.update(cx, |v, _| v.try_load_url("https://example.com/"))
                        .is_err(),
                    "email view refuses direct network navigation",
                );
                let selection = self.selection.clone();
                cx.subscribe(&view, move |_, _, event: &WebViewEvent, _| {
                    if let WebViewEvent::SelectionChanged(state) = event {
                        *selection.borrow_mut() = Some(state.text.clone());
                    }
                })
                .detach();
                self.selection_supported = view.read(cx).capabilities().selected_text;
                if self.selection_supported {
                    view.read(cx).select_all().expect("email select all");
                }
                self.next();
            }
            18 => {
                if self.selection_supported {
                    view.read(cx)
                        .selected_text()
                        .expect("email selected text query");
                } else {
                    self.check(
                        view.read(cx).selected_text().is_err(),
                        "mixed-script engine accurately refuses isolated selected-text query",
                    );
                }
                self.next();
            }
            19 => {
                let selection_ok = self.selection.borrow().as_deref().is_some_and(|text| {
                    text.contains("interoperability")
                        && text.contains("שלום")
                        && text.contains("中文")
                });
                if self.selection_supported {
                    self.check(
                        selection_ok,
                        "selected email text includes Unicode without script evaluation",
                    );
                }
                self.check(
                    view.read(cx).last_copied_text().is_none(),
                    "selection query does not emit a host clipboard copy",
                );
                view.read(cx)
                    .find("中文", FindOptions::default())
                    .expect("email find");
                self.next();
            }
            20 => {
                if view.read(cx).find_state().searching {
                    return;
                }
                self.check(
                    view.read(cx).find_state().matches > 0,
                    "native CJK find works with scripting disabled",
                );
                view.read(cx).clear_find().expect("clear search");
                let a = Document::html(
                    r#"<!doctype html><title>Restricted A</title><style>body{margin:0;background:rgb(20,90,140)} @import url('http://127.0.0.1:18765/raw-css');</style><p>Restricted native-policy fixture שלום 中文</p><img src='http://127.0.0.1:18765/raw-image'><iframe src='http://127.0.0.1:18765/raw-frame'></iframe><object data='http://127.0.0.1:18765/raw-object'></object><video poster='http://127.0.0.1:18765/raw-poster'></video><script>location.href='http://127.0.0.1:18765/raw-script'</script><link rel='preconnect' href='http://127.0.0.1:18765'><link rel='dns-prefetch' href='http://localhost'><form action='http://127.0.0.1:18765/raw-form'><input autofocus value='untrusted'></form>"#,
                );
                let other = Document::html("<title>Restricted B</title><p>Second message</p>");
                view.read(cx)
                    .load_document(other.clone())
                    .expect("load prepared B");
                self.documents = Some((a, other));
                self.next();
            }
            21 => {
                if view.read(cx).status().loading || view.read(cx).status().title != "Restricted B"
                {
                    return;
                }
                let before = view.read(cx).state().document_id;
                self.check(
                    view.read(cx)
                        .load_document(Document::html("bad\0body"))
                        .is_err(),
                    "invalid load is rejected before commit",
                );
                self.check(
                    view.read(cx).state().document_id == before,
                    "rejected load retains active document identity",
                );
                let bounds = view.read(cx).viewport_bounds();
                self.check(
                    view.read(cx)
                        .set_geometry(
                            Bounds::new(point(px(0.), px(0.)), size(px(20000.), px(20000.))),
                            bounds,
                        )
                        .is_err(),
                    "oversized viewport is rejected",
                );
                self.check(
                    view.read(cx).viewport_bounds() == bounds,
                    "rejected geometry retains viewport",
                );
                let doc = self.documents.as_ref().unwrap().0.clone();
                view.read(cx)
                    .load_document(doc)
                    .expect("load older prepared A after B");
                self.next();
            }
            22 => {
                let state = view.read(cx).state();
                if state.loading || state.title != "Restricted A" {
                    return;
                }
                let (a, other) = {
                    let (a, b) = self.documents.as_ref().unwrap();
                    (a.id(), b.id())
                };
                self.check(
                    state.document_id == Some(a),
                    "prepared documents load independently of ID allocation order",
                );
                self.check(
                    view.read(cx)
                        .provide_resource(
                            other,
                            "stale",
                            gpui_wpe::ResourceResponse::new(vec![1], "image/png").unwrap(),
                        )
                        .is_err(),
                    "old message resources are revoked",
                );
                self.check(
                    view.read(cx).status().error.is_empty(),
                    "raw restricted document renders without sanitizer dependency",
                );
                self.next();
            }
            23 => {
                let handle = view.read(cx).handle();
                view.update(cx, |view, cx| view.close(cx))
                    .expect("close view");
                self.check(
                    matches!(handle.set_zoom(1.2), Err(gpui_wpe::Error::Closed)),
                    "retained handle returns Closed",
                );
                view.update(cx, |view, cx| view.close(cx))
                    .expect("idempotent close");
                w.defer(cx, |w, cx| {
                    w.dispatch_keystroke(Keystroke::parse("x").unwrap(), cx);
                    w.dispatch_event(
                        PlatformInput::KeyUp(KeyUpEvent {
                            keystroke: Keystroke::parse("x").unwrap(),
                        }),
                        cx,
                    );
                });
                self.next();
            }
            24 => {
                self.check(true, "late keyboard events after close are harmless");
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
