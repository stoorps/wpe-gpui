//! A native Wayland WPE view hosted between GPUI content and its overlays.
mod engine;
pub mod error;
pub mod find;
pub mod geometry;
pub mod policy;
pub mod resource;
pub mod settings;
pub use engine::ResourceResponder;
pub use error::{Error, Result};
use gpui::*;
use raw_window_handle::{HasDisplayHandle, RawDisplayHandle, RawWindowHandle};
pub use resource::{Document, DocumentId, ResourceRequest, ResourceResponse};
use std::{
    cell::{Cell, RefCell},
    ops::Range,
    rc::Rc,
};
pub use wpe_bridge::{
    Action, EditingState, Failure as WebViewFailure, FailureKind, HitTest, LoadEvent, LoadPhase,
    Status,
};

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
    closed: Cell<bool>,
    clip: Cell<Bounds<Pixels>>,
    window: AnyWindowHandle,
    settings: settings::WebViewSettings,
    resource_policy: policy::ResourcePolicy,
    document: Cell<Option<DocumentId>>,
    epoch: Cell<u64>,
    zoom: Cell<f64>,
    navigation_handler: Option<NavigationHandler>,
    resource_handler: Option<ResourceHandler>,
    notify: RefCell<Option<NotifyHandler>>,
    pending_errors: RefCell<Vec<Error>>,
    instance: u64,
    geometry_accepted_us: Cell<i64>,
    resource_sizes: RefCell<std::collections::BTreeMap<String, usize>>,
    resource_liveness: RefCell<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    selection_request: Cell<Option<(u64, Option<DocumentId>)>>,
    resource_history: RefCell<
        std::collections::VecDeque<(DocumentId, std::collections::BTreeMap<String, usize>)>,
    >,
}
impl Native {
    fn input(&self, event: &PlatformInput) {
        if self.closed.get() || !self.visible.get() || self.blocked.get() {
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
                wpe_bridge::wake();
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
    fn install_resource(
        &self,
        id: DocumentId,
        key: &str,
        response: &ResourceResponse,
    ) -> Result<()> {
        resource::validate_key(key)?;
        if response.bytes.len() > self.settings.max_resource_bytes {
            return Err(Error::InvalidInput(
                "resource exceeds configured byte limit".into(),
            ));
        }
        let sizes = self.resource_sizes.borrow();
        let previous = sizes.get(key).copied().unwrap_or(0);
        let total = sizes.values().sum::<usize>() - previous + response.bytes.len();
        drop(sizes);
        if total > self.settings.max_total_resource_bytes {
            return Err(Error::InvalidInput(
                "resource aggregate exceeds configured limit".into(),
            ));
        }
        self.view
            .set_resource(id.get(), key, &response.bytes, &response.mime_type)
            .map_err(Error::from)?;
        self.resource_sizes
            .borrow_mut()
            .insert(key.into(), response.bytes.len());
        if let Some((_, sizes)) = self
            .resource_history
            .borrow_mut()
            .iter_mut()
            .find(|(document, _)| *document == id)
        {
            sizes.insert(key.into(), response.bytes.len());
        }
        Ok(())
    }
    fn apply_geometry(
        &self,
        document: Bounds<Pixels>,
        clip: Bounds<Pixels>,
        scale: f64,
    ) -> Result<()> {
        if self.closed.get() {
            return Err(Error::Closed);
        }
        if !scale.is_finite() || scale <= 0. {
            return Err(Error::InvalidInput("invalid scale".into()));
        }
        for value in [
            document.origin.x,
            document.origin.y,
            document.size.width,
            document.size.height,
            clip.origin.x,
            clip.origin.y,
            clip.size.width,
            clip.size.height,
        ] {
            if !f32::from(value).is_finite() {
                return Err(Error::InvalidInput("non-finite bounds".into()));
            }
        }
        if document.size.width < px(0.)
            || document.size.height < px(0.)
            || clip.size.width < px(0.)
            || clip.size.height < px(0.)
        {
            return Err(Error::InvalidInput("negative viewport or clip size".into()));
        }
        let width = (f32::from(document.size.width).round() as f64 * scale).ceil();
        let height = (f32::from(document.size.height).round() as f64 * scale).ceil();
        if width > 16384. || height > 16384. || width * height > 16_777_216. {
            return Err(Error::InvalidInput(
                "viewport exceeds renderer dimensions or pixel limit".into(),
            ));
        }
        let old = self.bounds.get();
        let accepted_us = wpe_bridge::monotonic_us();
        let geometry_changed = old.size != document.size || self.scale.get() != scale;
        let visible = document.intersect(&clip);
        let mapped = self.visible.get() && !visible.is_empty();
        if let Some(surface) = self.surface.borrow().as_ref() {
            let attachment = surface.platform_surface().map_err(Error::from)?;
            attachment
                .set_bounds(visible.to_device_pixels(scale as f32))
                .map_err(Error::from)?;
            attachment.set_visible(mapped).map_err(Error::from)?;
        }
        if geometry_changed {
            self.geometry_accepted_us.set(accepted_us);
        }
        self.bounds.set(document);
        self.clip.set(clip);
        self.scale.set(scale);
        self.view.geometry(
            f32::from(document.size.width).round() as i32,
            f32::from(document.size.height).round() as i32,
            scale,
            mapped,
        );
        self.view.set_clip(
            f32::from(visible.origin.x - document.origin.x).max(0.) as f64,
            f32::from(visible.origin.y - document.origin.y).max(0.) as f64,
            f32::from(visible.size.width).max(0.) as f64,
            f32::from(visible.size.height).max(0.) as f64,
        );
        Ok(())
    }
    fn validate_url(&self, url: &str) -> Result<()> {
        if self.closed.get() {
            return Err(Error::Closed);
        }
        if self.resource_policy == policy::ResourcePolicy::ProvidedOnly {
            return Err(Error::PolicyDenied(
                "this view accepts provided documents only".into(),
            ));
        }
        if !(url.starts_with("https://") || url.starts_with("http://"))
            || url.chars().any(|c| c.is_control())
        {
            return Err(Error::InvalidInput(
                "load_url supports HTTP(S); use Document/resources for local content".into(),
            ));
        }
        if let Some(handler) = &self.navigation_handler {
            let request = policy::NavigationRequest {
                url: url.into(),
                document_id: self.document.get(),
                user_gesture: None,
                new_window: false,
            };
            if handler(&request) != policy::NavigationDecision::Allow {
                return Err(Error::PolicyDenied(
                    "host navigation policy denied explicit load".into(),
                ));
            }
        }
        Ok(())
    }
    fn sync_document(&self) {
        if self.closed.get() {
            return;
        }
        let current = DocumentId::from_native(self.view.document_id());
        if self.document.get() != current {
            self.cancel_resources();
            self.document.set(current);
            *self.resource_sizes.borrow_mut() = self
                .resource_history
                .borrow()
                .iter()
                .find(|(id, _)| Some(*id) == current)
                .map(|(_, sizes)| sizes.clone())
                .unwrap_or_default();
        }
    }
    fn cancel_resources(&self) {
        self.selection_request.set(None);
        self.resource_liveness
            .borrow()
            .store(false, std::sync::atomic::Ordering::Release);
        *self.resource_liveness.borrow_mut() =
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        self.epoch.set(self.epoch.get() + 1);
    }
    fn stop_native(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.visible.set(false);
        self.cancel_resources();
        self.resource_liveness
            .borrow()
            .store(false, std::sync::atomic::Ordering::Release);
        self.cancel_input();
        if let Some(surface) = self.surface.borrow().as_ref() {
            let _ = surface
                .platform_surface()
                .and_then(|s| s.set_visible(false));
        }
        self.view.close();
        self.notify.borrow_mut().take();
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
impl Drop for Native {
    fn drop(&mut self) {
        self.stop_native();
        if let Some(surface) = self.surface.borrow_mut().take() {
            engine::release_surface(self.window, self.id, surface);
        }
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

/// A managed native WPE view. Construction never selects a workload profile.
pub struct WebView {
    native: Rc<Native>,
    focus: FocusHandle,
    marked: Option<String>,
    last_copy: Option<String>,
    bound: bool,
    state: WebViewState,
    last_find: find::FindState,
    closed_emitted: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct WebViewState {
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub closed: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub document_id: Option<DocumentId>,
    pub error: Option<Error>,
}
#[derive(Clone, Debug)]
pub enum WebViewEvent {
    StateChanged(WebViewState),
    NavigationRequested(policy::NavigationRequest),
    FindChanged(find::FindState),
    Error(Error),
    Closed,
    SelectionCopied(String),
    SelectionChanged(SelectionState),
    LoadChanged(LoadEvent),
    ContextMenuRequested(HitTest),
    Failure(WebViewFailure),
}
impl EventEmitter<WebViewEvent> for WebView {}
impl EventEmitter<DismissEvent> for WebView {}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectionState {
    pub document_id: Option<DocumentId>,
    pub text: String,
}

#[derive(Clone, Debug)]
pub struct WebViewCapabilities {
    pub native_wayland: bool,
    pub independent_resources: bool,
    pub find_in_page: bool,
    pub rich_clipboard: bool,
    pub accessibility: bool,
    /// WPE currently broadcasts its color preference process-wide.
    pub per_view_theme: bool,
    pub selected_text: bool,
    pub exact_caret_geometry: bool,
    pub software_fallback: bool,
}
impl Default for WebViewCapabilities {
    fn default() -> Self {
        Self {
            native_wayland: true,
            independent_resources: true,
            find_in_page: true,
            rich_clipboard: false,
            accessibility: false,
            per_view_theme: false,
            selected_text: false,
            exact_caret_geometry: false,
            software_fallback: false,
        }
    }
}
type NotifyHandler = Box<dyn Fn(&mut App)>;
type NavigationHandler = Rc<dyn Fn(&policy::NavigationRequest) -> policy::NavigationDecision>;
type ResourceHandler = Rc<dyn Fn(resource::ResourceRequest, ResourceResponder)>;

#[derive(Default)]
pub struct WebViewBuilder {
    url: Option<String>,
    document: Option<Document>,
    settings: settings::WebViewSettings,
    resource_policy: policy::ResourcePolicy,
    navigation_handler: Option<NavigationHandler>,
    resource_handler: Option<ResourceHandler>,
}
impl WebViewBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        self.url = Some(url.into());
        self.document = None;
        self
    }
    pub fn with_html(mut self, html: impl Into<String>) -> Self {
        self.document = Some(Document::html(html));
        self.url = None;
        self
    }
    pub fn with_document(mut self, document: Document) -> Self {
        self.document = Some(document);
        self.url = None;
        self
    }
    pub fn with_settings(mut self, settings: settings::WebViewSettings) -> Self {
        self.settings = settings;
        self
    }
    pub fn with_background_color(mut self, color: [f64; 4]) -> Self {
        self.settings.background = color;
        self
    }
    pub fn with_resource_policy(mut self, policy: policy::ResourcePolicy) -> Self {
        self.resource_policy = policy;
        self
    }
    pub fn with_navigation_handler(
        mut self,
        handler: impl Fn(&policy::NavigationRequest) -> policy::NavigationDecision + 'static,
    ) -> Self {
        self.navigation_handler = Some(Rc::new(handler));
        self
    }
    pub fn with_resource_provider(
        self,
        handler: impl Fn(resource::ResourceRequest, ResourceResponder) + 'static,
    ) -> Self {
        self.with_resource_handler(handler)
    }
    pub fn with_resource_handler(
        mut self,
        handler: impl Fn(resource::ResourceRequest, ResourceResponder) + 'static,
    ) -> Self {
        self.resource_handler = Some(Rc::new(handler));
        self
    }
}

#[derive(Clone)]
pub struct WebViewHandle {
    native: std::rc::Weak<Native>,
}
impl WebViewHandle {
    fn native(&self) -> Result<Rc<Native>> {
        let n = self.native.upgrade().ok_or(Error::Closed)?;
        if n.closed.get() {
            return Err(Error::Closed);
        }
        Ok(n)
    }
    pub fn load_url(&self, url: &str) -> Result<()> {
        let n = self.native()?;
        n.validate_url(url)?;
        n.document.set(None);
        n.cancel_resources();
        n.resource_sizes.borrow_mut().clear();
        n.view.load_uri(url);
        wpe_bridge::wake();
        Ok(())
    }
    pub fn set_zoom(&self, zoom: f64) -> Result<()> {
        let n = self.native()?;
        validate_zoom(zoom)?;
        n.zoom.set(zoom);
        n.view.zoom(zoom);
        Ok(())
    }
    pub fn close(&self, cx: &mut App) -> Result<()> {
        let n = self.native.upgrade().ok_or(Error::Closed)?;
        engine::close_native(n, cx);
        Ok(())
    }
}
impl WebView {
    pub fn build(builder: WebViewBuilder, window: &mut Window, cx: &mut App) -> Result<Self> {
        UI_THREAD.get_or_init(|| std::thread::current().id());
        assert_ui_thread();
        engine::ensure_creation_allowed()?;
        if builder.resource_policy == policy::ResourcePolicy::ProvidedOnly
            && builder.settings.javascript != settings::JavaScript::Disabled
        {
            return Err(Error::InvalidInput(
                "ProvidedOnly requires explicit JavaScript::Disabled".into(),
            ));
        }
        builder.settings.validate()?;
        let RawDisplayHandle::Wayland(display) = window
            .display_handle()
            .map_err(|e| Error::Native(format!("Display handle: {e:?}")))?
            .as_raw()
        else {
            return Err(Error::Unsupported("native Wayland display required"));
        };
        let composition = window.enable_window_composition().map_err(Error::from)?;
        let surface = composition.create_native_surface().map_err(Error::from)?;
        let raw = surface
            .platform_surface()
            .map_err(Error::from)?
            .platform_handle()
            .map_err(Error::from)?;
        let Some(RawWindowHandle::Wayland(raw)) = raw.downcast_ref::<RawWindowHandle>() else {
            composition
                .remove_surface(surface.id())
                .map_err(Error::from)?;
            return Err(Error::Unsupported("Wayland native composition surface"));
        };
        let config = wpe_bridge::Config {
            javascript: builder.settings.javascript == settings::JavaScript::Enabled,
            restricted_resources: builder.resource_policy == policy::ResourcePolicy::ProvidedOnly,
            mediate_navigation: builder.navigation_handler.is_some(),
            theme: match builder.settings.theme {
                settings::Theme::Auto => 0,
                settings::Theme::Light => 1,
                settings::Theme::Dark => 2,
            },
            background_rgba: builder.settings.background,
        };
        let view = match unsafe {
            wpe_bridge::View::new_with_config(
                display.display.as_ptr(),
                raw.surface.as_ptr(),
                &config,
            )
        } {
            Ok(v) => v,
            Err(e) => {
                let _ = composition.remove_surface(surface.id());
                return Err(e.into());
            }
        };
        let native = Rc::new(Native {
            selection_request: Cell::new(None),
            resource_history: RefCell::new(std::collections::VecDeque::new()),
            view,
            id: surface.id(),
            surface: RefCell::new(Some(surface)),
            window: window.window_handle(),
            bounds: Cell::new(Bounds::default()),
            clip: Cell::new(Bounds::default()),
            visible: Cell::new(true),
            scale: Cell::new(1.),
            blocked: Cell::new(false),
            clicked: Cell::new(false),
            buttons: Cell::new(0),
            closed: Cell::new(false),
            settings: builder.settings,
            resource_policy: builder.resource_policy,
            document: Cell::new(None),
            epoch: Cell::new(0),
            zoom: Cell::new(1.),
            navigation_handler: builder.navigation_handler,
            resource_handler: builder.resource_handler,
            pending_errors: RefCell::new(Vec::new()),
            notify: RefCell::new(None),
            instance: engine::next_instance(),
            geometry_accepted_us: Cell::new(0),
            resource_sizes: RefCell::new(std::collections::BTreeMap::new()),
            resource_liveness: RefCell::new(std::sync::Arc::new(
                std::sync::atomic::AtomicBool::new(true),
            )),
        });
        let weak = Rc::downgrade(&native);
        if let Err(error) = composition.on_input(native.id, move |event| {
            if let Some(n) = weak.upgrade() {
                n.input(event);
            }
            DispatchEventResult {
                propagate: false,
                default_prevented: false,
            }
        }) {
            native.stop_native();
            let _ = composition.remove_surface(native.id);
            native.surface.borrow_mut().take();
            return Err(error.into());
        }
        engine::register(&native, cx);
        let mut this = Self {
            native,
            focus: cx.focus_handle(),
            marked: None,
            last_copy: None,
            bound: false,
            state: WebViewState::default(),
            last_find: find::FindState::default(),
            closed_emitted: false,
        };
        let initial = if let Some(doc) = builder.document {
            this.load_document(doc)
        } else if let Some(url) = builder.url {
            this.try_load_url(&url)
        } else {
            Ok(())
        };
        if let Err(error) = initial {
            engine::close_native(this.native.clone(), cx);
            return Err(error);
        }
        Ok(this)
    }
    fn ensure_open(&self) -> Result<()> {
        if self.native.closed.get() {
            Err(Error::Closed)
        } else {
            Ok(())
        }
    }
    pub fn try_load_url(&mut self, url: &str) -> Result<()> {
        self.handle().load_url(url)
    }
    pub fn load_url(&mut self, url: &str) {
        if let Err(e) = self.try_load_url(url) {
            self.native.pending_errors.borrow_mut().push(e);
            wpe_bridge::wake();
        }
    }
    pub fn load_html(&self, html: &str) -> Result<()> {
        self.load_document(Document::html(html))
    }
    pub fn load_document(&self, document: Document) -> Result<()> {
        self.ensure_open()?;
        if document.html_source().len() > self.native.settings.max_document_bytes {
            return Err(Error::InvalidInput(
                "document exceeds configured byte limit".into(),
            ));
        }
        let mut total = 0usize;
        for (key, response) in document.resources() {
            resource::validate_key(key)?;
            if response.bytes.len() > self.native.settings.max_resource_bytes {
                return Err(Error::InvalidInput(
                    "resource exceeds configured byte limit".into(),
                ));
            }
            total = total
                .checked_add(response.bytes.len())
                .ok_or_else(|| Error::InvalidInput("resource size overflow".into()))?;
        }
        if total > self.native.settings.max_total_resource_bytes {
            return Err(Error::InvalidInput(
                "document resources exceed total limit".into(),
            ));
        }
        let resources = document
            .resources()
            .iter()
            .map(|(key, response)| wpe_bridge::DocumentResource {
                key,
                bytes: &response.bytes,
                mime: &response.mime_type,
            })
            .collect::<Vec<_>>();
        self.native
            .view
            .load_document_with_resources(document.id().get(), document.html_source(), &resources)
            .map_err(Error::from)?;
        self.native.cancel_resources();
        self.native.document.set(Some(document.id()));
        *self.native.resource_sizes.borrow_mut() = document
            .resources()
            .iter()
            .map(|(key, response)| (key.clone(), response.bytes.len()))
            .collect();
        {
            let mut history = self.native.resource_history.borrow_mut();
            history.retain(|(id, _)| *id != document.id());
            history.push_back((document.id(), self.native.resource_sizes.borrow().clone()));
            while history.len() > 8 {
                history.pop_front();
            }
        }
        wpe_bridge::wake();
        Ok(())
    }
    pub fn provide_resource(
        &self,
        id: DocumentId,
        key: &str,
        response: resource::ResourceResponse,
    ) -> Result<()> {
        self.ensure_open()?;
        if self.native.document.get() != Some(id) {
            return Err(Error::Cancelled);
        }
        self.native.install_resource(id, key, &response)
    }
    pub fn status(&self) -> Status {
        if self.native.closed.get() {
            Status::default()
        } else {
            self.native.view.status()
        }
    }
    pub fn state(&self) -> WebViewState {
        let mut state = self.state.clone();
        if self.native.closed.get() {
            state.loading = false;
            state.closed = true;
        }
        state
    }
    pub fn capabilities(&self) -> WebViewCapabilities {
        WebViewCapabilities {
            selected_text: !self.native.closed.get()
                && self.native.view.selection_query_supported(),
            ..Default::default()
        }
    }
    pub fn handle(&self) -> WebViewHandle {
        WebViewHandle {
            native: Rc::downgrade(&self.native),
        }
    }
    pub fn back(&mut self) -> Result<()> {
        self.ensure_open()?;
        self.native.view.action(Action::Back);
        Ok(())
    }
    pub fn forward(&mut self) -> Result<()> {
        self.ensure_open()?;
        self.native.view.action(Action::Forward);
        Ok(())
    }
    pub fn reload(&self) -> Result<()> {
        self.ensure_open()?;
        self.native.cancel_resources();
        self.native.view.action(Action::Reload);
        Ok(())
    }
    pub fn stop_loading(&self) -> Result<()> {
        self.ensure_open()?;
        self.native.view.action(Action::Stop);
        Ok(())
    }
    pub fn action(&self, action: Action, cx: &mut App) -> Result<()> {
        self.ensure_open()?;
        if matches!(action, Action::Reload) {
            return self.reload();
        }
        if matches!(action, Action::Copy | Action::Cut | Action::Paste)
            && !self.native.view.clipboard_available()
        {
            return Err(Error::Native("native clipboard is unavailable; restart application after a clipboard protocol failure".into()));
        }
        if matches!(action, Action::Paste) {
            self.native.view.set_clipboard(
                &cx.read_from_clipboard()
                    .and_then(|v| v.text())
                    .unwrap_or_default(),
            );
        }
        self.native.view.action(action);
        Ok(())
    }
    pub fn copy(&self, cx: &mut App) -> Result<()> {
        self.action(Action::Copy, cx)
    }
    pub fn cut(&self, cx: &mut App) -> Result<()> {
        self.action(Action::Cut, cx)
    }
    pub fn paste(&self, cx: &mut App) -> Result<()> {
        self.action(Action::Paste, cx)
    }
    pub fn undo(&self) -> Result<()> {
        self.ensure_open()?;
        self.native.view.action(Action::Undo);
        Ok(())
    }
    pub fn redo(&self) -> Result<()> {
        self.ensure_open()?;
        self.native.view.action(Action::Redo);
        Ok(())
    }

    pub fn select_all(&self) -> Result<()> {
        self.ensure_open()?;
        self.native.view.action(Action::SelectAll);
        Ok(())
    }
    /// Request selected text without changing the host clipboard. Completion is
    /// emitted as SelectionChanged. Supported only in engine sessions which have never enabled page scripting;
    /// ordinary Copy works in both configurations. One query may be outstanding
    /// per engine session; a timeout invalidates the shared clipboard.
    pub fn request_selected_text(&self) -> Result<()> {
        self.ensure_open()?;
        if !self.native.view.selection_query_supported() {
            return Err(Error::Unsupported(
                "side-effect-free selection queries require an engine session that has never enabled page scripting; use Copy",
            ));
        }
        self.native
            .view
            .request_selection_text()
            .map_err(Error::from)?;
        self.native
            .selection_request
            .set(Some((self.native.epoch.get(), self.native.document.get())));
        wpe_bridge::wake();
        Ok(())
    }
    /// Shorthand for request_selected_text; completion is an event, not a
    /// synchronous return value. Prefer the explicit request spelling in new code.
    pub fn selected_text(&self) -> Result<()> {
        self.request_selected_text()
    }
    pub fn hit_test(&self) -> Result<Option<HitTest>> {
        self.ensure_open()?;
        Ok(self.native.view.hit_test())
    }
    pub fn editing_state(&self) -> Result<EditingState> {
        self.ensure_open()?;
        Ok(self.native.view.editing_state())
    }
    /// Test-only process crash injection for validating host recovery UI.
    #[doc(hidden)]
    pub fn diagnostic_terminate_process(&self) -> Result<()> {
        self.ensure_open()?;
        self.native.view.terminate_process();
        Ok(())
    }
    pub fn last_copied_text(&self) -> Option<&str> {
        self.last_copy.as_deref()
    }
    pub fn paste_text(&self, text: &str) -> Result<()> {
        self.ensure_open()?;
        if !self.native.view.clipboard_available() {
            return Err(Error::Native("native clipboard is unavailable; restart application after a clipboard protocol failure".into()));
        }
        self.native.view.set_clipboard(text);
        self.native.view.action(Action::Paste);
        Ok(())
    }
    pub fn set_zoom(&self, value: f64) -> Result<()> {
        self.handle().set_zoom(value)
    }
    pub fn zoom(&self) -> f64 {
        self.native.zoom.get()
    }
    pub fn set_theme(&self, theme: settings::Theme) -> Result<()> {
        self.ensure_open()?;
        if theme != self.native.settings.theme {
            return Err(Error::Unsupported(
                "independent per-view color-scheme preference on WPE; use app-owned document styling",
            ));
        }
        Ok(())
    }
    pub fn find(&self, text: &str, options: find::FindOptions) -> Result<()> {
        self.ensure_open()?;
        self.native.view.find(text, options.case_sensitive);
        Ok(())
    }
    pub fn find_next(&self) -> Result<()> {
        self.ensure_open()?;
        self.native.view.find_next();
        Ok(())
    }
    pub fn find_previous(&self) -> Result<()> {
        self.ensure_open()?;
        self.native.view.find_previous();
        Ok(())
    }
    pub fn clear_find(&self) -> Result<()> {
        self.ensure_open()?;
        self.native.view.find_clear();
        Ok(())
    }
    pub fn find_state(&self) -> find::FindState {
        if self.native.closed.get() {
            return find::FindState::default();
        }
        let state = self.native.view.find_state();
        find::FindState {
            matches: state.matches,
            searching: state.pending,
        }
    }
    /// Trusted test/diagnostic operation. Not a native bridge exposed to page scripts.
    pub fn evaluate(&self, script: &str) -> Result<()> {
        self.ensure_open()?;
        if self.native.settings.javascript == settings::JavaScript::Disabled {
            return Err(Error::PolicyDenied(
                "diagnostic evaluation disabled for this view".into(),
            ));
        }
        self.native.view.evaluate(script);
        Ok(())
    }
    pub fn take_evaluation(&self) -> Option<String> {
        if self.native.closed.get() {
            None
        } else {
            self.native.view.take_evaluation()
        }
    }
    pub fn viewport_bounds(&self) -> Bounds<Pixels> {
        self.native.bounds.get()
    }
    pub fn metrics(&self) -> Result<wpe_bridge::Metrics> {
        self.ensure_open()?;
        Ok(self.native.view.metrics())
    }
    pub fn geometry_timestamp_us(&self) -> i64 {
        self.native.geometry_accepted_us.get()
    }
    /// Diagnostic GPU readback only; never required to render an email.
    pub fn sample_pixel(&self, x: i32, y: i32) -> Result<[u8; 4]> {
        self.ensure_open()?;
        self.native.view.sample_pixel(x, y).map_err(Error::from)
    }
    pub fn set_geometry(&self, document: Bounds<Pixels>, clip: Bounds<Pixels>) -> Result<()> {
        self.ensure_open()?;
        self.native
            .apply_geometry(document, clip, self.native.scale.get())
    }
    pub fn set_bounds(&self, rect: geometry::Rect) -> Result<()> {
        let document: Bounds<Pixels> = rect.into();
        self.set_geometry(document, document)
    }
    pub fn take_new_uri(&self) -> Option<String> {
        if self.native.closed.get() {
            None
        } else {
            self.native.view.take_new_uri()
        }
    }
    pub fn set_visible(&self, visible: bool) -> Result<()> {
        self.ensure_open()?;
        self.native.visible.set(visible);
        if !visible {
            self.native.cancel_input();
        }
        if let Some(surface) = self.native.surface.borrow().as_ref() {
            surface
                .platform_surface()
                .map_err(Error::from)?
                .set_visible(visible)
                .map_err(Error::from)?;
        }
        let b = self.native.bounds.get();
        self.native.view.geometry(
            f32::from(b.size.width).round() as i32,
            f32::from(b.size.height).round() as i32,
            self.native.scale.get(),
            visible,
        );
        wpe_bridge::wake();
        Ok(())
    }
    pub fn show(&mut self) {
        if let Err(e) = self.set_visible(true) {
            self.native.pending_errors.borrow_mut().push(e);
        }
    }
    pub fn hide(&mut self) {
        if let Err(e) = self.set_visible(false) {
            self.native.pending_errors.borrow_mut().push(e);
        }
    }
    pub fn visible(&self) -> bool {
        self.native.visible.get() && !self.native.closed.get()
    }
    pub fn bounds(&self) -> Bounds<Pixels> {
        self.native.bounds.get()
    }
    pub fn block_input(&self, blocked: bool) {
        if self.native.closed.get() {
            return;
        }
        if blocked && !self.native.blocked.replace(blocked) {
            self.native.cancel_input();
        }
        self.native.blocked.set(blocked);
    }
    pub fn close(&mut self, cx: &mut App) -> Result<()> {
        engine::close_native(self.native.clone(), cx);
        Ok(())
    }
    fn finish_close(&mut self, cx: &mut Context<Self>) {
        for error in self.native.pending_errors.borrow_mut().drain(..) {
            cx.emit(WebViewEvent::Error(error));
        }
        if !self.closed_emitted {
            self.closed_emitted = true;
            self.state.loading = false;
            self.state.closed = true;
            cx.emit(WebViewEvent::Closed);
            cx.notify();
        }
    }
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.native.closed.get() {
            self.finish_close(cx);
            return;
        }
        self.native.sync_document();
        if self.native.clicked.replace(false) && self.visible() && !self.native.blocked.get() {
            window.focus(&self.focus, cx);
        }
        self.native.view.focus(
            self.focus.is_focused(window)
                && window.is_window_active()
                && self.visible()
                && !self.native.blocked.get(),
        );
        if let Some(text) = self.native.view.take_clipboard() {
            self.last_copy = Some(text.clone());
            cx.emit(WebViewEvent::SelectionCopied(text.clone()));
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
        if let Some(result) = self.native.view.take_selection_text()
            && let Some((epoch, document_id)) = self.native.selection_request.take()
            && epoch == self.native.epoch.get()
        {
            match result {
                Ok(text) => cx.emit(WebViewEvent::SelectionChanged(SelectionState {
                    document_id,
                    text,
                })),
                Err(error) => cx.emit(WebViewEvent::Error(error.into())),
            }
        }
        while let Some(load) = self.native.view.take_load_event() {
            cx.emit(WebViewEvent::LoadChanged(load));
        }
        while let Some(context) = self.native.view.take_context_menu() {
            cx.emit(WebViewEvent::ContextMenuRequested(context));
        }
        while let Some(failure) = self.native.view.take_failure() {
            cx.emit(WebViewEvent::Failure(failure));
            cx.notify();
        }
        let status = self.native.view.status();
        let state = WebViewState {
            url: status.uri,
            title: status.title,
            loading: status.loading,
            closed: false,
            can_go_back: status.can_back,
            can_go_forward: status.can_forward,
            document_id: self.native.document.get(),
            error: (!status.error.is_empty()).then_some(Error::Native(status.error)),
        };
        if state != self.state {
            self.state = state.clone();
            cx.emit(WebViewEvent::StateChanged(state));
            cx.notify();
        }
        let find = self.find_state();
        if find != self.last_find {
            self.last_find = find.clone();
            cx.emit(WebViewEvent::FindChanged(find));
            cx.notify();
        }
        for error in self.native.pending_errors.borrow_mut().drain(..) {
            cx.emit(WebViewEvent::Error(error));
        }
        while let Some(navigation) = self.native.view.take_navigation() {
            let request = policy::NavigationRequest {
                url: navigation.uri,
                document_id: self.native.document.get(),
                user_gesture: Some(navigation.user_gesture),
                new_window: navigation.new_window,
            };
            let decision = self
                .native
                .navigation_handler
                .as_ref()
                .map(|h| h(&request))
                .unwrap_or(policy::NavigationDecision::Delegate);
            if navigation.token != 0 {
                self.native.view.resolve_navigation(
                    navigation.token,
                    decision == policy::NavigationDecision::Allow,
                );
            } else if decision == policy::NavigationDecision::Allow
                && self.native.resource_policy == policy::ResourcePolicy::Unrestricted
                && (request.url.starts_with("https://") || request.url.starts_with("http://"))
                && let Err(error) = self.try_load_url(&request.url)
            {
                cx.emit(WebViewEvent::Error(error));
            }
            if decision == policy::NavigationDecision::Delegate {
                cx.emit(WebViewEvent::NavigationRequested(request));
            }
        }
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.native.closed.get()
            || !self.visible()
            || !self.focus.is_focused(window)
            || self.native.blocked.get()
        {
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
                if let Err(error) = self.action(action, cx) {
                    cx.emit(WebViewEvent::Error(error));
                }
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.bound {
            self.bound = true;
            let weak = cx.weak_entity();
            let handle = window.window_handle();
            *self.native.notify.borrow_mut() = Some(Box::new(move |cx| {
                if handle
                    .update(cx, |_, window, cx| {
                        let _ = weak.update(cx, |view, cx| view.refresh(window, cx));
                    })
                    .is_err()
                {
                    let _ = weak.update(cx, |view, cx| {
                        if view.native.closed.get() {
                            view.finish_close(cx);
                        }
                    });
                }
            }));
            cx.on_release(|view, cx| engine::close_native(view.native.clone(), cx))
                .detach();
            let native = Rc::downgrade(&self.native);
            cx.on_focus(&self.focus, window, move |_, _, _| {
                if let Some(n) = native.upgrade()
                    && !n.closed.get()
                {
                    n.view.focus(true);
                }
            })
            .detach();
            let native = Rc::downgrade(&self.native);
            cx.on_blur(&self.focus, window, move |_, _, _| {
                if let Some(n) = native.upgrade()
                    && !n.closed.get()
                {
                    n.cancel_input();
                }
            })
            .detach();
            wpe_bridge::wake();
        }
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
                if !this.native.closed.get()
                    && this.visible()
                    && this.focus.is_focused(window)
                    && !this.native.blocked.get()
                {
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
        if self.native.closed.get() || !self.native.visible.get() {
            return;
        }
        if let Err(error) = self.native.apply_geometry(
            bounds,
            window.content_mask().bounds,
            window.scale_factor() as f64,
        ) {
            self.native.pending_errors.borrow_mut().push(error);
            wpe_bridge::wake();
        }
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
        if self.native.closed.get() || !self.native.visible.get() {
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
        if !self.native.closed.get() {
            self.native.view.text("", false);
        }
    }
    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        if !self.native.closed.get() && self.visible() && !self.native.blocked.get() {
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
        if !self.native.closed.get() && self.visible() && !self.native.blocked.get() {
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
/// Runtime identity and the native event loop are owned by the library.
pub fn runtime_info() -> wpe_bridge::RuntimeInfo {
    wpe_bridge::runtime_info()
}
fn validate_zoom(value: f64) -> Result<()> {
    if !value.is_finite() || !(0.5..=3.).contains(&value) {
        Err(Error::InvalidInput(
            "zoom must be finite and within 0.5..=3.0".into(),
        ))
    } else {
        Ok(())
    }
}

/// Monotonic microseconds shared with native trace timestamps.
pub fn monotonic_us() -> i64 {
    wpe_bridge::monotonic_us()
}

/// Diagnostic ownership counters. The App token keeps this operation on GPUI's
/// owning thread; counters describe this bridge, not all WebKit/driver objects.
pub fn live_metrics(_cx: &App) -> wpe_bridge::LiveMetrics {
    assert_ui_thread();
    unsafe { wpe_bridge::live_metrics() }
}
