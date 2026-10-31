//! A native Wayland WPE view hosted between GPUI content and its overlays.
use anyhow::{Result, anyhow, bail};
use gpui::*;
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle, RawWindowHandle};
use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
};
pub use wpe_bridge::{Action, Status};

pub const DEFAULT_URI: &str = "https://fullsphere.co.uk/html-emails/free-template/";

struct Native {
    // Order matters: the producer stops before the native surface is released.
    view: wpe_bridge::View,
    surface: RefCell<Option<WindowCompositionSurface>>,
    id: CompositionSurfaceId,
    bounds: Cell<Bounds<Pixels>>,
    visible: Cell<bool>,
    scale: Cell<f64>,
    blocked: Cell<bool>,
    clicked: Cell<bool>,
    buttons: Cell<u32>,
}
impl Native {
    fn input(&self, event: &PlatformInput) {
        if !self.visible.get() || self.blocked.get() {
            return;
        }
        let origin = self.bounds.get().origin;
        let position = |p: Point<Pixels>| {
            (
                f32::from(p.x - origin.x) as f64,
                f32::from(p.y - origin.y) as f64,
            )
        };
        match event {
            PlatformInput::MouseDown(e) => {
                let (x, y) = position(e.position);
                let button = button(e.button);
                if button == 0 {
                    return;
                }
                self.buttons.set(self.buttons.get() | button_mask(button));
                self.view.pointer(
                    0,
                    x,
                    y,
                    button,
                    modifiers(e.modifiers) | self.buttons.get(),
                    e.click_count as i32,
                );
                self.clicked.set(true);
            }
            PlatformInput::MouseUp(e) => {
                let (x, y) = position(e.position);
                let button = button(e.button);
                if button == 0 {
                    return;
                }
                self.buttons.set(self.buttons.get() & !button_mask(button));
                self.view.pointer(
                    1,
                    x,
                    y,
                    button,
                    modifiers(e.modifiers) | self.buttons.get(),
                    e.click_count as i32,
                );
            }
            PlatformInput::MouseMove(e) => {
                let (x, y) = position(e.position);
                self.view
                    .pointer(2, x, y, 0, modifiers(e.modifiers) | self.buttons.get(), 0);
            }
            PlatformInput::MouseExited(_) => self.view.pointer(3, 0., 0., 0, 0, 0),
            PlatformInput::Scroll(e) => {
                let (x, y) = position(e.position);
                let delta = e.delta.pixel_delta(px(24.));
                self.view.scroll(
                    x,
                    y,
                    f32::from(delta.x) as f64,
                    f32::from(delta.y) as f64,
                    modifiers(e.modifiers),
                );
            }
            _ => {}
        }
    }
    fn cancel_input(&self) {
        self.clicked.set(false);
        let buttons = self.buttons.replace(0);
        for b in 1..=3 {
            if buttons & button_mask(b) != 0 {
                self.view.pointer(1, 0., 0., b, 0, 1);
            }
        }
        self.view.text("", false);
        self.view.focus(false);
    }
}
fn button(button: MouseButton) -> i32 {
    match button {
        MouseButton::Left => 1,
        MouseButton::Middle => 2,
        MouseButton::Right => 3,
        _ => 0,
    }
}
fn button_mask(button: i32) -> u32 {
    match button {
        1..=5 => 1 << (7 + button),
        _ => 0,
    }
}
fn modifiers(m: Modifiers) -> u32 {
    u32::from(m.control)
        | (u32::from(m.shift) << 1)
        | (u32::from(m.alt) << 2)
        | (u32::from(m.platform) << 3)
}
fn key_name(key: &str) -> &str {
    match key {
        "enter" => "Return",
        "backspace" => "BackSpace",
        "escape" => "Escape",
        "space" => "space",
        "pageup" => "Prior",
        "pagedown" => "Next",
        other => other,
    }
}

static UI_THREAD: std::sync::OnceLock<std::thread::ThreadId> = std::sync::OnceLock::new();
fn assert_ui_thread() {
    if let Some(thread) = UI_THREAD.get() {
        assert_eq!(
            *thread,
            std::thread::current().id(),
            "WPE must stay on its creating UI thread"
        );
    }
}

pub struct WebView {
    native: Rc<Native>,
    focus: FocusHandle,
    marked: Option<String>,
    closed: bool,
    last_copy: Option<String>,
    email: Cell<bool>,
}
impl WebView {
    pub fn new(window: &mut Window, cx: &mut App) -> Result<Self> {
        UI_THREAD.get_or_init(|| std::thread::current().id());
        assert_ui_thread();
        let RawDisplayHandle::Wayland(display) = window
            .display_handle()
            .map_err(|e| anyhow!("Display handle: {e:?}"))?
            .as_raw()
        else {
            bail!("browser-demo requires native Wayland");
        };
        let composition = window.enable_window_composition()?;
        let surface = composition.create_native_surface()?;
        let handle = surface.platform_surface()?.platform_handle()?;
        let Some(RawWindowHandle::Wayland(handle)) = handle.downcast_ref::<RawWindowHandle>()
        else {
            bail!("Expected a Wayland native slot");
        };
        // SAFETY: Native owns a strong surface handle and drops View first.
        // All calls are on GPUI's UI thread; GPUI owns and dispatches the display.
        let view = match unsafe {
            wpe_bridge::View::new(display.display.as_ptr(), handle.surface.as_ptr())
        } {
            Ok(view) => view,
            Err(error) => {
                composition.remove_surface(surface.id())?;
                return Err(error);
            }
        };
        let native = Rc::new(Native {
            view,
            id: surface.id(),
            surface: RefCell::new(Some(surface)),
            bounds: Cell::new(Bounds::default()),
            visible: Cell::new(true),
            scale: Cell::new(1.),
            blocked: Cell::new(false),
            clicked: Cell::new(false),
            buttons: Cell::new(0),
        });
        let weak = Rc::downgrade(&native);
        composition.on_input(native.id, move |event| {
            if let Some(native) = weak.upgrade() {
                native.input(event);
            }
            DispatchEventResult {
                propagate: false,
                default_prevented: false,
            }
        })?;
        Ok(Self {
            native,
            focus: cx.focus_handle(),
            marked: None,
            closed: false,
            last_copy: None,
            email: Cell::new(false),
        })
    }
    pub fn load_uri(&self, uri: &str) {
        self.email.set(false);
        self.native.view.load_uri(uri);
    }
    pub fn load_email_html(&self, html: &str) {
        self.email.set(true);
        self.native
            .view
            .load_trusted_html(&sanitize_email(html), "message://body/", true);
    }
    pub fn is_email(&self) -> bool {
        self.email.get()
    }
    pub fn status(&self) -> Status {
        self.native.view.status()
    }
    pub fn action(&self, action: Action, cx: &mut App) {
        if matches!(action, Action::Paste) {
            self.native.view.set_clipboard(
                &cx.read_from_clipboard()
                    .and_then(|i| i.text())
                    .unwrap_or_default(),
            );
        }
        self.native.view.action(action);
    }
    pub fn last_copied_text(&self) -> Option<&str> {
        self.last_copy.as_deref()
    }
    pub fn paste_text(&self, text: &str) {
        self.native.view.set_clipboard(text);
        self.native.view.action(Action::Paste);
    }
    pub fn zoom(&self, value: f64) {
        self.native.view.zoom(value);
    }
    /// Diagnostic hook for deterministic interoperability checks on trusted fixtures.
    pub fn evaluate(&self, script: &str) {
        self.native.view.evaluate(script);
    }
    pub fn take_evaluation(&self) -> Option<String> {
        self.native.view.take_evaluation()
    }
    pub fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.native.bounds.get()
    }
    pub fn take_new_uri(&self) -> Option<String> {
        self.native.view.take_new_uri()
    }
    pub fn tick(&mut self, window: &mut Window, cx: &mut App) {
        if self.closed {
            return;
        }
        if self.native.clicked.replace(false)
            && self.native.visible.get()
            && !self.native.blocked.get()
        {
            window.focus(&self.focus, cx);
        }
        self.native.view.focus(
            self.focus.is_focused(window)
                && window.is_window_active()
                && self.native.visible.get()
                && !self.native.blocked.get(),
        );
        if let Some(text) = self.native.view.take_clipboard() {
            self.last_copy = Some(text.clone());
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
    pub fn set_active(&self, active: bool) -> Result<()> {
        self.native.visible.set(active);
        if !active {
            self.native.cancel_input();
        }
        if let Some(surface) = self.native.surface.borrow().as_ref() {
            surface.platform_surface()?.set_visible(active)?;
        }
        let bounds = self.native.bounds.get();
        self.native.view.geometry(
            f32::from(bounds.size.width).round() as i32,
            f32::from(bounds.size.height).round() as i32,
            self.native.scale.get(),
            active,
        );
        Ok(())
    }
    pub fn block_input(&self, blocked: bool) {
        if blocked && !self.native.blocked.replace(blocked) {
            self.native.cancel_input();
        }
        self.native.blocked.set(blocked);
    }
    pub fn close(&mut self, window: &mut Window) -> Result<()> {
        if !self.closed {
            self.set_active(false)?;
            self.native.view.close();
            window
                .enable_window_composition()?
                .remove_surface(self.native.id)?;
            self.native.surface.borrow_mut().take();
            self.closed = true;
        }
        Ok(())
    }
    /// Release native resources when the hosting window has already closed.
    pub fn dispose_after_window_close(&mut self) {
        if self.closed {
            return;
        }
        self.native.visible.set(false);
        self.native.cancel_input();
        self.native.view.close();
        self.native.surface.borrow_mut().take();
        self.closed = true;
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) || self.native.blocked.get() {
            return;
        }
        let k = &event.keystroke;
        if k.modifiers.control || k.modifiers.platform {
            let action = match k.key.as_str() {
                "a" => Some(Action::SelectAll),
                "c" => Some(Action::Copy),
                "x" => Some(Action::Cut),
                "v" => Some(Action::Paste),
                "z" => Some(if k.modifiers.shift {
                    Action::Redo
                } else {
                    Action::Undo
                }),
                "y" => Some(Action::Redo),
                _ => None,
            };
            if let Some(action) = action {
                self.action(action, cx);
                cx.stop_propagation();
                return;
            }
        }
        // Printable text is delivered by EntityInputHandler, not twice via keys.
        if k.key_char.is_none()
            || k.modifiers.control
            || k.modifiers.platform
            || matches!(
                k.key.as_str(),
                "enter" | "tab" | "backspace" | "delete" | "escape"
            )
        {
            self.native
                .view
                .key(key_name(&k.key), modifiers(k.modifiers), true);
            cx.stop_propagation();
        }
    }
}
impl Focusable for WebView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for WebView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let cursor = match self.status().cursor.as_str() {
            "text" | "vertical-text" => CursorStyle::IBeam,
            "pointer" => CursorStyle::PointingHand,
            _ => CursorStyle::Arrow,
        };
        div()
            .size_full()
            .track_focus(&self.focus)
            .cursor(cursor)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus, cx);
                }),
            )
            .on_key_down(cx.listener(Self::key))
            .on_key_up(cx.listener(|this, e: &KeyUpEvent, window, cx| {
                if this.focus.is_focused(window) && !this.native.blocked.get() {
                    this.native.view.key(
                        key_name(&e.keystroke.key),
                        modifiers(e.keystroke.modifiers),
                        false,
                    );
                    cx.stop_propagation();
                }
            }))
            .child(ViewElement {
                entity: cx.entity(),
                native: self.native.clone(),
            })
    }
}
struct ViewElement {
    entity: Entity<WebView>,
    native: Rc<Native>,
}
impl IntoElement for ViewElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for ViewElement {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        _: &mut App,
    ) {
        if !self.native.visible.get() {
            return;
        }
        self.native.bounds.set(bounds);
        self.native.scale.set(window.scale_factor() as f64);
        if let Some(surface) = self.native.surface.borrow().as_ref()
            && let Ok(attachment) = surface.platform_surface()
            && let Err(error) =
                attachment.set_bounds(bounds.to_device_pixels(window.scale_factor()))
        {
            eprintln!("WPE bounds: {error:#}");
        }
        self.native.view.geometry(
            f32::from(bounds.size.width).round() as i32,
            f32::from(bounds.size.height).round() as i32,
            window.scale_factor() as f64,
            self.native.visible.get(),
        );
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.native.visible.get() {
            return;
        }
        let focus = self.entity.read(cx).focus.clone();
        window.handle_input(
            &focus,
            ElementInputHandler::new(bounds, self.entity.clone()),
            cx,
        );
    }
}
impl EntityInputHandler for WebView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        None
    }
    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|s| 0..s.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, _: &mut Context<Self>) {
        self.marked = None;
        self.native.view.text("", false);
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        if !self.native.blocked.get() {
            self.marked = None;
            self.native.view.text(text, false);
        }
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        if !self.native.blocked.get() {
            self.marked = Some(text.into());
            self.native.view.text(text, true);
        }
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        Some(Bounds {
            origin: bounds.origin,
            size: size(px(1.), px(24.)),
        })
    }
    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}
/// Call once per UI tick, before polling view status/clipboard.
pub fn pump() {
    assert_ui_thread();
    unsafe { wpe_bridge::pump() }
}
/// Restrictive offline message mode: sanitize first, then prepend a policy the
/// supplied HTML cannot relax. No remote-content opt-in is offered yet.
pub fn sanitize_email(html: &str) -> String {
    let cleaned = ammonia::Builder::default().clean(html).to_string();
    format!(
        "<!doctype html><meta charset=\"utf-8\"><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; img-src data:; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-src 'none'; object-src 'none'\"><title>Email fixture</title><body>{cleaned}</body>"
    )
}
/// Close all views, then release the shared engine synchronously inside
/// `App::on_app_quit`, before GPUI removes windows and Wayland callback state.
/// Calling while views remain live returns false and leaves the engine intact.
pub fn shutdown() -> bool {
    assert_ui_thread();
    unsafe { wpe_bridge::shutdown() }
}

#[cfg(test)]
mod tests {
    use super::sanitize_email;
    #[test]
    fn hostile_email_is_sanitized_before_policy_is_added() {
        let html = sanitize_email(
            r#"<meta http-equiv="Content-Security-Policy" content="default-src *"><base href="https://evil.test"><script>alert(1)</script><img src="https://evil.test/pixel" onerror="alert(2)"><a href="javascript:alert(3)">bad</a><p>Keep this</p>"#,
        );
        assert_eq!(html.matches("Content-Security-Policy").count(), 1);
        assert!(!html.contains("<script"));
        assert!(!html.contains("onerror"));
        assert!(!html.contains("javascript:"));
        assert!(!html.contains("<base "));
        assert!(html.contains("Keep this"));
    }
}
