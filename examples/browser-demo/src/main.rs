mod resize_benchmark;
mod runtime_benchmark;
mod self_test;
mod text_input;
use email_renderer::{Limits, PreparedEmail, RemoteConsent, RemoteFetcher};
use gpui::prelude::*;
use gpui::*;
use gpui_wpe::{
    Action, Document, FailureKind, HitTest, Status, WebView, WebViewBuilder, WebViewEvent,
    WebViewFailure,
};
use gpui_wpe::{
    find::FindOptions,
    policy::ResourcePolicy,
    settings::{JavaScript, WebViewSettings},
};
use std::collections::BTreeMap;
const DEFAULT_URI: &str = "https://fullsphere.co.uk/html-emails/free-template/";
use std::time::{Duration, Instant};
use text_input::TextInput;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Command {
    Back,
    Forward,
    Reload,
    Stop,
    Home,
    NewTab,
    CloseTab,
    NextTab,
    PreviousTab,
    Copy,
    Cut,
    Paste,
    SelectAll,
    Undo,
    Redo,
    ZoomIn,
    ZoomOut,
    ZoomReset,
    InputLab,
    EmailFixture,
    EmailLight,
    EmailDark,
    Find,
    FindNext,
    FindPrevious,
    RemoteImages,
    PlainText,
    About,
    Modal,
    Popover,
    Submenu,
    OpenExternal,
    InspectLink,
    CopyLink,
    Retry,
    Quit,
}
#[derive(Clone, Copy, PartialEq)]
enum Overlay {
    None,
    Menu(usize),
    About,
    Modal,
    Popover,
    ExternalLink,
    Context,
    Compose,
}
struct EmailTab {
    document: Document,
    prepared: Option<PreparedEmail>,
    consent: Option<RemoteConsent>,
    fetching: bool,
    report: String,
    dark: bool,
}
impl Drop for EmailTab {
    fn drop(&mut self) {
        if let Some(consent) = &self.consent {
            consent.revoke();
        }
    }
}
#[derive(Clone)]
struct ViewConfiguration {
    settings: WebViewSettings,
    resources: ResourcePolicy,
}
impl ViewConfiguration {
    fn builder(&self, document: Option<Document>, uri: &str) -> gpui_wpe::Result<WebViewBuilder> {
        let builder = WebViewBuilder::new()
            .with_settings(self.settings.clone())
            .with_resource_policy(self.resources);
        if let Some(document) = document {
            Ok(builder.with_document(document))
        } else {
            Ok(builder.with_url(
                normalize_uri(uri).map_err(|error| gpui_wpe::Error::InvalidInput(error.into()))?,
            ))
        }
    }
}
struct Tab {
    view: Entity<WebView>,
    status: Status,
    zoom: f64,
    id: usize,
    email: Option<EmailTab>,
    failure: Option<WebViewFailure>,
    configuration: ViewConfiguration,
    documents: BTreeMap<gpui_wpe::DocumentId, Document>,
    target_uri: String,
}
struct Browser {
    tabs: Vec<Tab>,
    active: usize,
    next_id: usize,
    url: Entity<TextInput>,
    modal_input: Entity<TextInput>,
    find_input: Entity<TextInput>,
    show_find: bool,
    find_matches: u32,
    last_find: String,
    fallback: Option<String>,
    email_dark: bool,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    overlay: Overlay,
    submenu: bool,
    menu_index: usize,
    error: Option<String>,
    pending_link: Option<String>,
    context_hit: Option<HitTest>,
    context_position: Point<Pixels>,
    compose_to: Entity<TextInput>,
    compose_subject: Entity<TextInput>,
    compose_body: Entity<TextInput>,
    started: Instant,
    self_test: Option<self_test::SelfTest>,
    benchmark: Option<resize_benchmark::Benchmark>,
    runtime_benchmark: Option<runtime_benchmark::Benchmark>,
}
fn button(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .px_3()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .bg(rgb(0xe7edf3))
        .hover(|s| s.bg(rgb(0xd1e0ee)))
        .child(label.into())
}
fn panel() -> Div {
    div()
        .bg(rgb(0xffffff))
        .border_1()
        .border_color(rgb(0xc7d3df))
        .rounded_lg()
        .shadow_lg()
}
fn menu_items(menu: usize) -> Vec<(&'static str, Command)> {
    use Command::*;
    match menu {
        0 => vec![
            ("New tab      Ctrl+T", NewTab),
            ("Close tab    Ctrl+W", CloseTab),
            ("Home", Home),
            ("Input interoperability lab", InputLab),
            ("Reload       Ctrl+R", Reload),
            ("Stop loading", Stop),
            ("Quit         Ctrl+Q", Quit),
        ],
        1 => vec![
            ("Undo         Ctrl+Z", Undo),
            ("Redo         Ctrl+Shift+Z", Redo),
            ("Cut          Ctrl+X", Cut),
            ("Copy         Ctrl+C", Copy),
            ("Paste        Ctrl+V", Paste),
            ("Select all   Ctrl+A", SelectAll),
            ("Find in page Ctrl+F", Find),
        ],
        2 => vec![
            ("Zoom in", ZoomIn),
            ("Zoom out", ZoomOut),
            ("Actual size", ZoomReset),
            ("Email reading surface: Light", EmailLight),
            ("Email reading surface: Dark", EmailDark),
            ("Reload page", Reload),
            ("Go back", Back),
            ("Go forward", Forward),
            ("Input interoperability lab", InputLab),
            ("Show popover", Popover),
            ("Overlay Test  →", Submenu),
            ("Open test modal", Modal),
            ("About this prototype", About),
        ],
        3 => vec![
            ("New tab", NewTab),
            ("Close tab", CloseTab),
            ("Next tab     Ctrl+Tab", NextTab),
            ("Previous tab Ctrl+Shift+Tab", PreviousTab),
        ],
        _ => vec![
            ("Input interoperability lab", InputLab),
            ("Offline email fixture", EmailFixture),
            ("Overlay test modal", Modal),
            ("Overlay test popover", Popover),
            ("About browser-demo", About),
        ],
    }
}
impl Browser {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let url = cx.new(|cx| TextInput::new(DEFAULT_URI, cx));
        let modal_input = cx.new(|cx| TextInput::new("GPUI modal input — paste here", cx));
        let mut this = Self {
            tabs: vec![],
            active: 0,
            next_id: 0,
            url,
            modal_input,
            find_input: cx.new(|cx| TextInput::new("", cx)),
            show_find: false,
            find_matches: 0,
            last_find: String::new(),
            fallback: None,
            email_dark: false,
            focus: cx.focus_handle(),
            previous_focus: None,
            overlay: Overlay::None,
            submenu: false,
            menu_index: 0,
            error: None,
            pending_link: None,
            context_hit: None,
            context_position: point(px(0.), px(0.)),
            compose_to: cx.new(|cx| TextInput::new("", cx)),
            compose_subject: cx.new(|cx| TextInput::new("", cx)),
            compose_body: cx.new(|cx| TextInput::new("", cx)),
            started: Instant::now(),
            self_test: std::env::args()
                .any(|a| a == "--self-test")
                .then(self_test::SelfTest::default),
            benchmark: std::env::args()
                .any(|a| a == "--benchmark-resize")
                .then(resize_benchmark::Benchmark::default),
            runtime_benchmark: std::env::args()
                .any(|arg| arg == "--benchmark-runtime")
                .then(runtime_benchmark::Benchmark::default),
        };
        let initial = if this.runtime_benchmark.is_some() {
            "demo://runtime/".into()
        } else if this.benchmark.is_some() {
            "demo://resize/".into()
        } else if this.self_test.is_some() {
            "demo://editing/".into()
        } else {
            std::env::args()
                .skip(1)
                .find(|s| !s.starts_with("--"))
                .unwrap_or_else(|| DEFAULT_URI.into())
        };
        this.add_tab(&initial, window, cx);
        // The library owns engine readiness. Only deterministic diagnostics need a timer.
        if this.self_test.is_some() || this.benchmark.is_some() || this.runtime_benchmark.is_some()
        {
            cx.spawn_in(window, async move |this, cx| {
                loop {
                    let Ok(delay) = this.update(cx, |this, _| {
                        this.runtime_benchmark.as_ref().map_or(
                            Duration::from_millis(10),
                            runtime_benchmark::Benchmark::delay,
                        )
                    }) else {
                        break;
                    };
                    cx.background_executor().timer(delay).await;
                    if this
                        .update_in(cx, |this, window, cx| this.tick(window, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .detach();
        } else if std::env::var_os("BROWSER_DEMO_SMOKE").is_some() {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor()
                    .timer(Duration::from_secs(12))
                    .await;
                let _ = this.update_in(cx, |this, _, cx| this.finish_smoke(cx));
            })
            .detach();
        }
        this
    }
    fn add_tab(&mut self, uri: &str, window: &mut Window, cx: &mut Context<Self>) {
        let configuration = configuration_for_uri(uri);
        let document = demo_document(uri);
        let result = configuration
            .builder(document.clone(), uri)
            .and_then(|builder| WebView::build(builder, window, cx))
            .map(|view| cx.new(|_| view));
        match result {
            Ok(view) => {
                if let Some(tab) = self.tabs.get(self.active) {
                    let _ = tab.view.read(cx).set_visible(false);
                }
                let id = self.next_id;
                self.next_id += 1;
                self.subscribe_view(id, &view, window, cx);
                self.tabs.push(Tab {
                    view,
                    status: Status {
                        uri: uri.into(),
                        ..Default::default()
                    },
                    zoom: 1.,
                    id,
                    email: None,
                    failure: None,
                    configuration,
                    documents: document
                        .map(|document| BTreeMap::from([(document.id(), document)]))
                        .unwrap_or_default(),
                    target_uri: uri.into(),
                });
                self.active = self.tabs.len() - 1;
                self.fallback = None;
                self.last_find.clear();
                self.url.update(cx, |input, cx| input.set_text(uri, cx));
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Cannot create WPE view: {error:#}")),
        }
        cx.notify();
    }
    fn subscribe_view(
        &mut self,
        id: usize,
        view: &Entity<WebView>,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        cx.subscribe_in(
            view,
            window,
            move |this, emitter, event: &WebViewEvent, window, cx| {
                let Some(index) = this
                    .tabs
                    .iter()
                    .position(|tab| tab.id == id && tab.view.entity_id() == emitter.entity_id())
                else {
                    return;
                };
                match event {
                    WebViewEvent::StateChanged(state) => {
                        if state.url.starts_with("https://") || state.url.starts_with("http://") {
                            this.tabs[index].target_uri = state.url.clone();
                        }
                        let status = emitter.read(cx).status();
                        if index == this.active
                            && status.uri != this.tabs[index].status.uri
                            && !this.url.focus_handle(cx).is_focused(window)
                        {
                            this.url
                                .update(cx, |input, cx| input.set_text(&status.uri, cx));
                        }
                        this.tabs[index].status = status;
                    }
                    WebViewEvent::FindChanged(state) if index == this.active => {
                        this.find_matches = state.matches
                    }
                    WebViewEvent::NavigationRequested(request) => {
                        if request.user_gesture == Some(true) && request.url.starts_with("mailto:")
                        {
                            this.pending_link = Some(request.url.clone());
                            this.set_overlay(Overlay::ExternalLink, window, cx);
                        } else if this.tabs[index].email.is_some() {
                            // No automatic external navigation from message content.
                            if request.user_gesture == Some(true)
                                && (request.url.starts_with("https://")
                                    || request.url.starts_with("http://"))
                            {
                                this.pending_link = Some(request.url.clone());
                                this.set_overlay(Overlay::ExternalLink, window, cx);
                            }
                        } else if request.new_window {
                            this.add_tab(&request.url, window, cx);
                        } else {
                            let result = load_tab(&mut this.tabs[index], &request.url, cx);
                            if let Err(error) = result {
                                this.error = Some(error.to_string());
                            }
                        }
                    }
                    WebViewEvent::ContextMenuRequested(hit) if index == this.active => {
                        let origin = emitter.read(cx).viewport_bounds().origin;
                        this.context_position = origin + point(px(hit.x as f32), px(hit.y as f32));
                        this.context_hit = Some(hit.clone());
                        this.set_overlay(Overlay::Context, window, cx);
                    }
                    WebViewEvent::Failure(failure) => {
                        this.tabs[index].failure = Some(failure.clone())
                    }
                    WebViewEvent::Error(error) => this.error = Some(error.to_string()),
                    _ => {}
                }
                cx.notify();
            },
        )
        .detach();
    }
    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() || index == self.active {
            return;
        }
        self.set_overlay(Overlay::None, window, cx);
        let _ = self.tabs[self.active].view.read(cx).set_visible(false);
        self.active = index;
        self.fallback = None;
        self.last_find.clear();
        let tab = &self.tabs[index];
        let _ = tab.view.read(cx).set_visible(true);
        self.url
            .update(cx, |input, cx| input.set_text(&tab.status.uri, cx));
        window.focus(&tab.view.focus_handle(cx), cx);
        cx.notify();
    }
    fn set_overlay(&mut self, overlay: Overlay, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay == Overlay::None && overlay != Overlay::None {
            self.previous_focus = window.focused(cx);
        }
        self.overlay = overlay;
        self.submenu = false;
        self.menu_index = 0;
        for tab in &self.tabs {
            tab.view.read(cx).block_input(overlay != Overlay::None);
        }
        match overlay {
            Overlay::None => {
                if let Some(focus) = self.previous_focus.take() {
                    window.focus(&focus, cx);
                }
            }
            Overlay::Modal => window.focus(&self.modal_input.focus_handle(cx), cx),
            Overlay::Compose => window.focus(&self.compose_to.focus_handle(cx), cx),
            _ => window.focus(&self.focus, cx),
        }
        cx.notify();
    }
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Diagnostic snapshot; normal browser state arrives through WebViewEvent.
        for tab in &mut self.tabs {
            tab.status = tab.view.read(cx).status();
        }
        if let Some(mut test) = self.self_test.take() {
            test.tick(self, window, cx);
            self.self_test = Some(test);
        }
        if let Some(mut benchmark) = self.benchmark.take() {
            benchmark.tick(self, window, cx);
            self.benchmark = Some(benchmark);
        }
        if let Some(mut benchmark) = self.runtime_benchmark.take() {
            benchmark.tick(self, window, cx);
            self.runtime_benchmark = Some(benchmark);
        }
    }
    fn finish_smoke(&mut self, cx: &mut Context<Self>) {
        for tab in &mut self.tabs {
            tab.status = tab.view.read(cx).status();
            eprintln!(
                "SMOKE uri={} title={:?} frames={} error={:?}",
                tab.status.uri, tab.status.title, tab.status.frames, tab.status.error
            );
        }
        if self.error.is_some()
            || self.tabs.is_empty()
            || self.tabs.iter().any(|tab| {
                tab.status.frames == 0 || tab.status.loading || !tab.status.error.is_empty()
            })
        {
            eprintln!("SMOKE failed: the page did not finish loading and rendering cleanly");
            self_test::FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        cx.quit();
    }
    fn editing_command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        let name = match command {
            Command::Copy => "Copy",
            Command::Cut => "Cut",
            Command::Paste => "Paste",
            Command::SelectAll => "SelectAll",
            _ => "",
        };
        if self.url.focus_handle(cx).is_focused(window) {
            self.url
                .update(cx, |input, cx| input.edit(name, window, cx));
        } else if self.compose_to.focus_handle(cx).is_focused(window) {
            self.compose_to
                .update(cx, |input, cx| input.edit(name, window, cx));
        } else if self.compose_subject.focus_handle(cx).is_focused(window) {
            self.compose_subject
                .update(cx, |input, cx| input.edit(name, window, cx));
        } else if self.compose_body.focus_handle(cx).is_focused(window) {
            self.compose_body
                .update(cx, |input, cx| input.edit(name, window, cx));
        } else if self.find_input.focus_handle(cx).is_focused(window) {
            self.find_input
                .update(cx, |input, cx| input.edit(name, window, cx));
        } else if self.modal_input.focus_handle(cx).is_focused(window) {
            self.modal_input
                .update(cx, |input, cx| input.edit(name, window, cx));
        } else if let Some(tab) = self.tabs.get(self.active) {
            let action = match command {
                Command::Copy => Action::Copy,
                Command::Cut => Action::Cut,
                Command::Paste => Action::Paste,
                Command::SelectAll => Action::SelectAll,
                Command::Undo => Action::Undo,
                _ => Action::Redo,
            };
            if let Err(error) = tab.view.update(cx, |view, cx| view.action(action, cx)) {
                self.error = Some(error.to_string());
            }
        }
    }
    fn command(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        if command == Command::Submenu {
            self.submenu = !self.submenu;
            cx.notify();
            return;
        }
        self.set_overlay(Overlay::None, window, cx);
        match command {
            Command::OpenExternal => {
                if let Some(uri) = self.pending_link.take() {
                    if let Some((to, subject, body)) = compose_intent(&uri) {
                        self.compose_to
                            .update(cx, |input, cx| input.set_text(&to, cx));
                        self.compose_subject
                            .update(cx, |input, cx| input.set_text(&subject, cx));
                        self.compose_body
                            .update(cx, |input, cx| input.set_text(&body, cx));
                        self.set_overlay(Overlay::Compose, window, cx);
                    } else if uri.starts_with("https://") || uri.starts_with("http://") {
                        self.add_tab(&uri, window, cx);
                    } else {
                        self.error = Some(
                            "This demo handles HTTP(S) links and mailto compose intents".into(),
                        );
                    }
                }
            }
            Command::CopyLink => {
                if let Some(hit) = &self.context_hit {
                    cx.write_to_clipboard(ClipboardItem::new_string(hit.link_uri.clone()));
                }
            }
            Command::InspectLink => {
                if let Some(hit) = &self.context_hit {
                    self.pending_link = Some(hit.link_uri.clone());
                    self.set_overlay(Overlay::ExternalLink, window, cx);
                }
            }
            Command::Retry => {
                if self
                    .tabs
                    .get(self.active)
                    .and_then(|tab| tab.failure.as_ref())
                    .is_some_and(|failure| failure.kind == FailureKind::Renderer)
                {
                    self.rebuild_active(window, cx);
                } else if let Some(tab) = self.tabs.get_mut(self.active) {
                    if tab
                        .failure
                        .as_ref()
                        .is_some_and(|failure| failure.kind == FailureKind::Clipboard)
                    {
                        self.error =
                            Some("Restart browser-demo to restore clipboard access".into());
                        return;
                    }
                    match tab.view.read(cx).reload() {
                        Ok(()) => {
                            tab.failure = None;
                            self.error = None;
                        }
                        Err(error) => self.error = Some(error.to_string()),
                    }
                }
            }
            Command::NewTab => self.add_tab(DEFAULT_URI, window, cx),
            Command::CloseTab => {
                if !self.tabs.is_empty() {
                    let tab = self.tabs.remove(self.active);
                    tab.view.update(cx, |view, cx| {
                        if let Err(e) = view.close(cx) {
                            eprintln!("Close tab: {e:#}");
                        }
                    });
                    self.active = self.active.min(self.tabs.len().saturating_sub(1));
                    if self.tabs.is_empty() {
                        self.add_tab(DEFAULT_URI, window, cx);
                    } else {
                        let tab = &self.tabs[self.active];
                        let _ = tab.view.read(cx).set_visible(true);
                        self.url.update(cx, |i, cx| i.set_text(&tab.status.uri, cx));
                        window.focus(&tab.view.focus_handle(cx), cx);
                    }
                }
            }
            Command::NextTab | Command::PreviousTab => {
                if !self.tabs.is_empty() {
                    let n = self.tabs.len();
                    let index = (self.active
                        + if command == Command::NextTab {
                            1
                        } else {
                            n - 1
                        })
                        % n;
                    self.activate(index, window, cx);
                }
            }
            Command::Home => self.load_active(DEFAULT_URI, window, cx),
            Command::InputLab => self.load_active("demo://editing/", window, cx),
            Command::EmailFixture => self.email_fixture(window, cx),
            Command::EmailLight | Command::EmailDark => {
                self.email_dark = command == Command::EmailDark;
                self.email_fixture(window, cx);
            }
            Command::RemoteImages => self.load_remote_images(window, cx),
            Command::PlainText => {
                self.fallback = self
                    .tabs
                    .get(self.active)
                    .and_then(|tab| tab.email.as_ref())
                    .and_then(|email| email.prepared.as_ref())
                    .map(|mail| mail.fallback_text.clone());
                if let Some(tab) = self.tabs.get(self.active) {
                    let _ = tab.view.read(cx).set_visible(false);
                }
            }
            Command::Find => {
                self.show_find = true;
                window.focus(&self.find_input.focus_handle(cx), cx);
            }
            Command::FindNext => {
                self.search(false, cx);
            }
            Command::FindPrevious => {
                self.search(true, cx);
            }
            Command::About => self.set_overlay(Overlay::About, window, cx),
            Command::Modal => self.set_overlay(Overlay::Modal, window, cx),
            Command::Popover => self.set_overlay(Overlay::Popover, window, cx),
            Command::Quit => cx.quit(),
            Command::Copy
            | Command::Cut
            | Command::Paste
            | Command::SelectAll
            | Command::Undo
            | Command::Redo => self.editing_command(command, window, cx),
            _ => {
                if let Some(tab) = self.tabs.get_mut(self.active) {
                    let result = tab.view.update(cx, |view, cx| match command {
                        Command::Back => view.back(),
                        Command::Forward => view.forward(),
                        Command::Reload => view.reload(),
                        Command::Stop => view.stop_loading(),
                        Command::ZoomIn | Command::ZoomOut | Command::ZoomReset => {
                            tab.zoom = match command {
                                Command::ZoomIn => (tab.zoom + 0.1).min(3.),
                                Command::ZoomOut => (tab.zoom - 0.1).max(0.5),
                                _ => 1.,
                            };
                            view.set_zoom(tab.zoom)
                        }
                        _ => {
                            let _ = cx;
                            Ok(())
                        }
                    });
                    if let Err(error) = result {
                        self.error = Some(error.to_string());
                    }
                }
            }
        }
        cx.notify();
    }
    fn rebuild_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let state = tab.view.read(cx).state();
        let document = if let Some(email) = tab.email.as_mut() {
            if let Some(consent) = email.consent.take() {
                consent.revoke();
            }
            email.report = "View recreated; remote image consent revoked".into();
            Some(email.document.clone())
        } else if let Some(id) = state.document_id {
            let Some(document) = tab.documents.get(&id).cloned() else {
                self.error=Some("Cannot recreate an app document not retained by this demo; navigate to a fixture again".into());
                return;
            };
            Some(document)
        } else if tab.target_uri.starts_with("demo://") {
            tab.documents.values().next_back().cloned()
        } else {
            None
        };
        let builder = match tab.configuration.builder(document, &tab.target_uri) {
            Ok(builder) => builder,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        // Renderer failure may retain exceptional GPU work; stop it before replacement.
        let _ = tab.view.update(cx, |view, cx| view.close(cx));
        let view = match WebView::build(builder, window, cx) {
            Ok(view) => cx.new(|_| view),
            Err(error) => {
                self.error = Some(format!("Cannot recreate view: {error}"));
                return;
            }
        };
        let _ = view.read(cx).set_zoom(tab.zoom);
        let id = tab.id;
        tab.view = view.clone();
        tab.status = Status::default();
        tab.failure = None;
        self.subscribe_view(id, &view, window, cx);
        window.focus(&view.focus_handle(cx), cx);
        self.error = None;
        cx.notify();
    }
    fn load_active(&mut self, uri: &str, window: &mut Window, cx: &mut Context<Self>) {
        // A restricted message view can never become a network-enabled browser.
        if self
            .tabs
            .get(self.active)
            .is_some_and(|tab| tab.email.is_some())
        {
            let previous = self.active;
            self.add_tab(uri, window, cx);
            if self.active != previous {
                self.error = None;
            }
            return;
        }
        if let Some(tab) = self.tabs.get_mut(self.active) {
            match load_tab(tab, uri, cx) {
                Ok(()) => {
                    self.error = None;
                    window.focus(&tab.view.focus_handle(cx), cx);
                }
                Err(error) => self.error = Some(error.to_string()),
            }
        }
    }
    fn start_search(&mut self, cx: &mut Context<Self>) {
        let text = self.find_input.read(cx).text();
        self.last_find = text.clone();
        if let Some(tab) = self.tabs.get(self.active)
            && let Err(error) = tab.view.read(cx).find(&text, FindOptions::default())
        {
            self.error = Some(error.to_string());
        }
        cx.notify();
    }
    fn search(&mut self, previous: bool, cx: &mut Context<Self>) {
        if self.find_input.read(cx).text() != self.last_find {
            self.start_search(cx);
            return;
        }
        if let Some(tab) = self.tabs.get(self.active) {
            let view = tab.view.read(cx);
            let result = if previous {
                view.find_previous()
            } else {
                view.find_next()
            };
            if let Err(error) = result {
                self.error = Some(error.to_string());
            }
        }
        cx.notify();
    }
    fn email_fixture(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let limits = Limits::default();
        let document = Document::html("");
        let mut inline = BTreeMap::new();
        let hero = email_renderer::InlineResource::new(
            vec![
                137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 32, 0, 0, 0,
                12, 8, 6, 0, 0, 0, 3, 20, 191, 185, 0, 0, 0, 33, 73, 68, 65, 84, 120, 156, 99, 80,
                46, 91, 246, 127, 32, 49, 195, 168, 3, 70, 67, 96, 52, 4, 70, 67, 96, 52, 4, 148,
                7, 184, 40, 6, 0, 185, 115, 93, 46, 95, 237, 107, 94, 0, 0, 0, 0, 73, 69, 78, 68,
                174, 66, 96, 130,
            ],
            "image/png",
            &limits,
        )
        .expect("original valid fixture image");
        inline.insert("hero@example.test".into(), hero);
        let prepared = match email_renderer::prepare_email(
            document.id().get(),
            include_str!("../../../fixtures/email.html"),
            "Email rendering interoperability. Original local newsletter with inline images, links and selectable text.",
            &inline,
            &limits,
        ) {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        let document = match email_document(document, &prepared, self.email_dark) {
            Ok(value) => value,
            Err(error) => {
                self.error = Some(error.to_string());
                return;
            }
        };
        let settings = WebViewSettings {
            javascript: JavaScript::Disabled,
            max_resource_bytes: limits.max_resource_bytes,
            max_total_resource_bytes: limits.max_total_resource_bytes,
            background: if self.email_dark {
                [0.09, 0.13, 0.18, 1.]
            } else {
                [1., 1., 1., 1.]
            },
            ..Default::default()
        };
        let view = match WebView::build(
            WebViewBuilder::new()
                .with_settings(settings.clone())
                .with_resource_policy(ResourcePolicy::ProvidedOnly)
                .with_document(document.clone()),
            window,
            cx,
        ) {
            Ok(view) => cx.new(|_| view),
            Err(error) => {
                self.error = Some(format!("Email view unavailable: {error}"));
                self.fallback = Some(prepared.fallback_text);
                return;
            }
        };
        let report = format!(
            "{} remote images blocked; JavaScript and engine networking disabled",
            prepared.remote_images.len()
        );
        let id = if let Some(old) = self.tabs.get(self.active) {
            old.id
        } else {
            let id = self.next_id;
            self.next_id += 1;
            id
        };
        self.subscribe_view(id, &view, window, cx);
        let tab = Tab {
            view,
            status: Status::default(),
            zoom: 1.,
            id,
            failure: None,
            configuration: ViewConfiguration {
                settings,
                resources: ResourcePolicy::ProvidedOnly,
            },
            documents: BTreeMap::new(),
            target_uri: "Local sanitized email".into(),
            email: Some(EmailTab {
                document,
                prepared: Some(prepared),
                consent: None,
                fetching: false,
                report,
                dark: self.email_dark,
            }),
        };
        if self.tabs.is_empty() {
            self.tabs.push(tab);
            self.active = 0;
        } else {
            let old = std::mem::replace(&mut self.tabs[self.active], tab);
            let _ = old.view.update(cx, |view, cx| view.close(cx));
        }
        self.error = None;
        self.fallback = None;
        self.last_find.clear();
        self.url
            .update(cx, |input, cx| input.set_text("Local sanitized email", cx));
        cx.notify();
    }
    fn load_remote_images(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get_mut(self.active) else {
            return;
        };
        let Some(email) = tab.email.as_mut() else {
            return;
        };
        if email.fetching {
            return;
        }
        let Some(mut prepared) = email.prepared.take() else {
            return;
        };
        let document_id = email.document.id();
        let tab_id = tab.id;
        let consent = RemoteConsent::for_document(document_id.get());
        email.consent = Some(consent.clone());
        email.fetching = true;
        email.report = "Fetching approved images through the application worker…".into();
        cx.spawn_in(window, async move |this, cx| {
            let (prepared, result) = cx
                .background_executor()
                .spawn(async move {
                    let result = prepared
                        .approve_remote_images(&consent, &RemoteFetcher::new(Limits::default()));
                    (prepared, result)
                })
                .await;
            let _ = this.update_in(cx, |this, _, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
                    return;
                };
                let Some(email) = tab.email.as_mut() else {
                    return;
                };
                if email.document.id() != document_id {
                    return;
                }
                if email.consent.as_ref().is_none_or(RemoteConsent::is_revoked) {
                    // Retain only preparation metadata after cancellation. No new bytes
                    // enter the recreated view until another explicit consent action.
                    if email.prepared.is_none() {
                        email.prepared = Some(prepared);
                        email.fetching = false;
                        email.report =
                            "Remote images cancelled; approve again to display them".into();
                        cx.notify();
                    }
                    return;
                }
                email.fetching = false;
                email.report = match result {
                    Ok(report) => format!(
                        "{} images loaded; {} denied or failed",
                        report.loaded,
                        report.failed.len()
                    ),
                    Err(error) => error.to_string(),
                };
                match email_document(email.document.clone(), &prepared, email.dark) {
                    Ok(document) => match tab.view.read(cx).load_document(document.clone()) {
                        Ok(()) => email.document = document,
                        Err(error) => email.report = error.to_string(),
                    },
                    Err(error) => email.report = error.to_string(),
                }
                email.prepared = Some(prepared);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn navigate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.url.read(cx).text();
        match normalize_uri(&input) {
            Ok(uri) => {
                self.load_active(&uri, window, cx);
            }
            Err(error) => self.error = Some(error.into()),
        }
        cx.notify();
    }
    fn key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &event.keystroke;
        if k.key == "escape" {
            if self.overlay != Overlay::None {
                self.set_overlay(Overlay::None, window, cx);
                cx.stop_propagation();
                return;
            }
            if self.show_find {
                self.show_find = false;
                if let Some(tab) = self.tabs.get(self.active) {
                    let _ = tab.view.read(cx).clear_find();
                    window.focus(&tab.view.focus_handle(cx), cx);
                }
                cx.notify();
                cx.stop_propagation();
                return;
            }
            if self.url.focus_handle(cx).is_focused(window) {
                if let Some(tab) = self.tabs.get(self.active) {
                    self.url.update(cx, |i, cx| i.set_text(&tab.status.uri, cx));
                    window.focus(&tab.view.focus_handle(cx), cx);
                }
                cx.stop_propagation();
                return;
            }
        }
        if let Overlay::Menu(menu) = self.overlay {
            let items = menu_items(menu);
            match k.key.as_str() {
                "down" => self.menu_index = (self.menu_index + 1) % items.len(),
                "up" => self.menu_index = (self.menu_index + items.len() - 1) % items.len(),
                "right" => {
                    if items[self.menu_index].1 == Command::Submenu {
                        self.submenu = true;
                    } else {
                        self.overlay = Overlay::Menu((menu + 1) % 5);
                        self.menu_index = 0;
                    }
                }
                "left" => {
                    self.overlay = Overlay::Menu((menu + 4) % 5);
                    self.menu_index = 0;
                    self.submenu = false;
                }
                "enter" => {
                    self.command(
                        if self.submenu {
                            Command::Modal
                        } else {
                            items[self.menu_index].1
                        },
                        window,
                        cx,
                    );
                }
                _ => {}
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if matches!(
            self.overlay,
            Overlay::Modal
                | Overlay::About
                | Overlay::Popover
                | Overlay::ExternalLink
                | Overlay::Compose
                | Overlay::Context
        ) {
            return;
        }
        if k.modifiers.control {
            let cmd = match k.key.as_str() {
                "t" => Some(Command::NewTab),
                "w" => Some(Command::CloseTab),
                "r" => Some(Command::Reload),
                "q" => Some(Command::Quit),
                "f" => Some(Command::Find),
                "tab" => Some(if k.modifiers.shift {
                    Command::PreviousTab
                } else {
                    Command::NextTab
                }),
                _ => None,
            };
            if let Some(cmd) = cmd {
                self.command(cmd, window, cx);
                cx.stop_propagation();
                return;
            }
            if k.key == "l" {
                window.focus(&self.url.focus_handle(cx), cx);
                self.url.update(cx, |i, cx| i.select_everything(window, cx));
                cx.stop_propagation();
                return;
            }
        }
        if k.key == "enter" && self.find_input.focus_handle(cx).is_focused(window) {
            self.search(k.modifiers.shift, cx);
            cx.stop_propagation();
            return;
        }
        if k.key == "enter" && self.url.focus_handle(cx).is_focused(window) {
            self.navigate(window, cx);
            cx.stop_propagation();
        }
    }
    fn menu(&self, menu: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let mut dropdown = panel()
            .id("menu-panel")
            .absolute()
            .top(px(32.))
            .left(px(8. + menu as f32 * 64.))
            .w(px(288.))
            .p_2()
            .occlude()
            .on_mouse_down_out(
                cx.listener(|this, _, window, cx| this.set_overlay(Overlay::None, window, cx)),
            )
            .on_scroll(|_, _, cx| cx.stop_propagation());
        for (index, (label, command)) in menu_items(menu).into_iter().enumerate() {
            dropdown = dropdown.child(
                div()
                    .id(("menu-item", index))
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .cursor_pointer()
                    .when(index == self.menu_index, |s| s.bg(rgb(0xe5eff8)))
                    .hover(|s| s.bg(rgb(0xd5e8f5)))
                    .on_mouse_move(cx.listener(move |this, _, _, cx| {
                        if this.menu_index != index {
                            this.menu_index = index;
                            cx.notify();
                        }
                    }))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.command(command, window, cx)),
                    )
                    .child(label),
            );
        }
        deferred(dropdown)
    }
}
impl Render for Browser {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self
            .tabs
            .get(self.active)
            .map(|t| t.status.clone())
            .unwrap_or_default();
        let zoom = self.tabs.get(self.active).map(|t| t.zoom).unwrap_or(1.);
        let mut menubar = div()
            .flex()
            .gap_1()
            .h(px(32.))
            .items_center()
            .px_2()
            .bg(rgb(0xe7edf3));
        for (i, label) in ["File", "Edit", "View", "Tabs", "Help"]
            .into_iter()
            .enumerate()
        {
            menubar = menubar.child(
                div()
                    .id(("menubar", i))
                    .px_3()
                    .py_1()
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(0xcbdbe9)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.set_overlay(
                            if this.overlay == Overlay::Menu(i) {
                                Overlay::None
                            } else {
                                Overlay::Menu(i)
                            },
                            window,
                            cx,
                        )
                    }))
                    .child(label),
            );
        }
        let mut tabs = div()
            .flex()
            .gap_1()
            .px_2()
            .py_1()
            .h(px(38.))
            .bg(rgb(0xdce5ef));
        for (index, tab) in self.tabs.iter().enumerate() {
            let title = if tab.status.title.is_empty() {
                "New tab".to_string()
            } else {
                tab.status.title.chars().take(25).collect()
            };
            tabs = tabs.child(
                div()
                    .id(("tab", tab.id))
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_1()
                    .rounded_t_md()
                    .cursor_pointer()
                    .bg(rgb(if index == self.active {
                        0xffffff
                    } else {
                        0xcbd8e5
                    }))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.activate(index, window, cx)),
                    )
                    .child(format!(
                        "{}{}",
                        if tab.status.loading { "◌ " } else { "" },
                        title
                    ))
                    .child(
                        div()
                            .id(("close-tab", tab.id))
                            .px_1()
                            .hover(|s| s.bg(rgb(0xeebbbb)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.activate(index, window, cx);
                                this.command(Command::CloseTab, window, cx);
                            }))
                            .child("×"),
                    ),
            );
        }
        tabs = tabs.child(button("new-tab", "+").on_click(
            cx.listener(|this, _, window, cx| this.command(Command::NewTab, window, cx)),
        ));
        let toolbar = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .bg(rgb(0xffffff))
            .border_b_1()
            .border_color(rgb(0xcbd5e1))
            .child(
                button("back", "←")
                    .opacity(if status.can_back { 1. } else { 0.35 })
                    .on_click(cx.listener(|this, _, w, cx| this.command(Command::Back, w, cx))),
            )
            .child(
                button("forward", "→")
                    .opacity(if status.can_forward { 1. } else { 0.35 })
                    .on_click(cx.listener(|this, _, w, cx| this.command(Command::Forward, w, cx))),
            )
            .child(
                button("reload", if status.loading { "Stop" } else { "Reload" }).on_click(
                    cx.listener(|this, _, w, cx| {
                        this.command(
                            if this.tabs.get(this.active).is_some_and(|t| t.status.loading) {
                                Command::Stop
                            } else {
                                Command::Reload
                            },
                            w,
                            cx,
                        )
                    }),
                ),
            )
            .child(
                button("home", "Home")
                    .on_click(cx.listener(|this, _, w, cx| this.command(Command::Home, w, cx))),
            )
            .child(div().flex_1().min_w(px(100.)).child(self.url.clone()))
            .child(button("go", "Go").on_click(cx.listener(Self::navigate_click)))
            .child(
                button("zoom-out", "−")
                    .on_click(cx.listener(|this, _, w, cx| this.command(Command::ZoomOut, w, cx))),
            )
            .child(format!("{:.0}%", zoom * 100.))
            .child(
                button("zoom-in", "+")
                    .on_click(cx.listener(|this, _, w, cx| this.command(Command::ZoomIn, w, cx))),
            )
            .child(
                button("modal", "Test modal")
                    .on_click(cx.listener(|this, _, w, cx| this.command(Command::Modal, w, cx))),
            );
        let mut root = div()
            .id("browser-demo")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0xf2f6fa))
            .text_color(rgb(0x20364a))
            .font_family("Noto Sans")
            .text_size(px(14.))
            .track_focus(&self.focus)
            .capture_key_down(cx.listener(Self::key))
            .child(menubar)
            .child(tabs)
            .child(toolbar);
        if self.show_find {
            root = root.child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .py_2()
                    .child(div().w(px(280.)).child(self.find_input.clone()))
                    .child(
                        button("find-start", "Find")
                            .on_click(cx.listener(|this, _, _, cx| this.start_search(cx))),
                    )
                    .child(button("find-previous", "Previous").on_click(
                        cx.listener(|this, _, w, cx| this.command(Command::FindPrevious, w, cx)),
                    ))
                    .child(button("find-next", "Next").on_click(
                        cx.listener(|this, _, w, cx| this.command(Command::FindNext, w, cx)),
                    ))
                    .child(format!("{} matches", self.find_matches)),
            );
        }
        if let Some(email) = self
            .tabs
            .get(self.active)
            .and_then(|tab| tab.email.as_ref())
        {
            root =
                root.child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .px_3()
                        .py_2()
                        .bg(rgb(0xe6f1e8))
                        .child(
                            button(
                                "remote-images",
                                if email.fetching {
                                    "Loading images…"
                                } else {
                                    "Load remote images for this message"
                                },
                            )
                            .on_click(cx.listener(|this, _, w, cx| {
                                this.command(Command::RemoteImages, w, cx)
                            })),
                        )
                        .child(button("plain-text", "Plain text fallback").on_click(
                            cx.listener(|this, _, w, cx| this.command(Command::PlainText, w, cx)),
                        ))
                        .child(email.report.clone()),
                );
        }
        if let Some(failure) = self
            .tabs
            .get(self.active)
            .and_then(|tab| tab.failure.as_ref())
        {
            root = root.child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_3()
                    .bg(rgb(0xffe4d7))
                    .child(if failure.kind==FailureKind::Clipboard {format!("Clipboard unavailable: {}. Restart browser-demo to restore clipboard access.",failure.message)}else{format!("{:?}: {}",failure.kind,failure.message)})
                    .when(failure.kind!=FailureKind::Clipboard,|row|row.child(button("retry-view", if failure.kind==FailureKind::Renderer {"Recreate this view"}else{"Retry this view"}).on_click(
                        cx.listener(|this, _, w, cx| this.command(Command::Retry, w, cx)),
                    ))),
            );
        }
        if let Some(error) = self.error.as_ref().or(if status.error.is_empty() {
            None
        } else {
            Some(&status.error)
        }) {
            root = root.child(div().p_2().bg(rgb(0xffe4d7)).child(error.clone()));
        }
        root = if let Some(text) = &self.fallback {
            root.child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .p_6()
                    .id("plain-text-fallback")
                    .overflow_y_scroll()
                    .child(text.clone()),
            )
        } else {
            root.child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .children(self.tabs.get(self.active).map(|t| t.view.clone())),
            )
        };
        root=root
            .child(div().px_3().py_1().bg(rgb(0xe7edf3)).text_size(px(12.)).child(format!("WPE · native Wayland  |  {}  |  {} frames  |  Ctrl+L URL · Ctrl+T tab · View → overlay tests",if status.loading{"Loading"}else{"Ready"},status.frames)));
        if let Overlay::Menu(menu) = self.overlay {
            root = root.child(self.menu(menu, cx));
        }
        if self.submenu {
            root = root.child(
                deferred(
                    panel()
                        .id("submenu")
                        .absolute()
                        .left(px(425.))
                        .top(px(300.))
                        .w(px(230.))
                        .p_3()
                        .occlude()
                        .child(button("sub-modal", "Open modal").on_click(
                            cx.listener(|this, _, w, cx| this.command(Command::Modal, w, cx)),
                        ))
                        .child(button("sub-popover", "Open popover").on_click(
                            cx.listener(|this, _, w, cx| this.command(Command::Popover, w, cx)),
                        )),
                )
                .with_priority(2),
            );
        }
        if matches!(
            self.overlay,
            Overlay::Modal | Overlay::About | Overlay::ExternalLink
        ) {
            let modal = self.overlay == Overlay::Modal;
            let external = self.overlay == Overlay::ExternalLink;
            let mailto = self
                .pending_link
                .as_ref()
                .is_some_and(|uri| uri.starts_with("mailto:"));
            root=root.child(deferred(div().id("modal-backdrop").absolute().inset_0().flex().items_center().justify_center().bg(rgba(0x102a4690)).occlude()
                .on_scroll(|_,_,cx|cx.stop_propagation())
                .on_mouse_down(MouseButton::Left,cx.listener(|this,_,w,cx|this.set_overlay(Overlay::None,w,cx)))
                .child(panel().id("modal-panel").w(px(480.)).p_6().flex().flex_col().gap_4().occlude()
                    .on_mouse_down(MouseButton::Left,|_,_,cx|cx.stop_propagation())
                    .child(div().text_size(px(23.)).child(if modal{"GPUI overlay test"}else if external{"Open external link?"}else{"About browser-demo"}))
                    .child(if modal{"This dialog, shadow, and translucent backdrop are drawn by GPUI above WPE. Type or paste below; the page underneath must receive no input."}else{"A WPEPlatform WebKit view inside gpui-fast. All browser chrome, tabs, menus and dialogs are GPUI."})
                    .when(modal,|s|s.child(self.modal_input.clone()))
                    .when(external,|s|s.child(self.pending_link.clone().unwrap_or_default()).child(button("open-external",if mailto {"Review compose intent"} else {"Open in browser tab"}).on_click(cx.listener(|this,_,w,cx|this.command(Command::OpenExternal,w,cx)))))
                    .child(button("close-modal","Close · Escape").on_click(cx.listener(|this,_,w,cx|this.set_overlay(Overlay::None,w,cx)))))).with_priority(3));
        }
        if self.overlay == Overlay::Context {
            let bounds = window.viewport_size();
            let position = point(
                self.context_position
                    .x
                    .min((bounds.width - px(280.)).max(px(8.)))
                    .max(px(8.)),
                self.context_position
                    .y
                    .min((bounds.height - px(300.)).max(px(8.)))
                    .max(px(8.)),
            );
            let hit = self.context_hit.clone().unwrap_or_default();
            let mut menu = panel()
                .id("web-context-menu")
                .absolute()
                .left(position.x)
                .top(position.y)
                .w(px(272.))
                .p_2()
                .flex()
                .flex_col()
                .gap_1()
                .occlude()
                .on_mouse_down_out(
                    cx.listener(|this, _, w, cx| this.set_overlay(Overlay::None, w, cx)),
                )
                .on_scroll(|_, _, cx| cx.stop_propagation());
            if hit.selection || hit.editing.can_copy {
                menu = menu
                    .child(button("context-copy", "Copy selection").on_click(
                        cx.listener(|this, _, w, cx| this.command(Command::Copy, w, cx)),
                    ));
            }
            if hit.editable {
                menu =
                    menu.child(button("context-paste", "Paste").on_click(
                        cx.listener(|this, _, w, cx| this.command(Command::Paste, w, cx)),
                    ));
            }
            if !hit.link_uri.is_empty() {
                menu = menu
                    .child(button("context-copy-link", "Copy link address").on_click(
                        cx.listener(|this, _, w, cx| this.command(Command::CopyLink, w, cx)),
                    ))
                    .child(button("context-inspect-link", "Inspect link…").on_click(
                        cx.listener(|this, _, w, cx| this.command(Command::InspectLink, w, cx)),
                    ));
            }
            menu = menu.child(
                button("context-find", "Find in page")
                    .on_click(cx.listener(|this, _, w, cx| this.command(Command::Find, w, cx))),
            );
            root = root.child(deferred(menu).with_priority(4));
        }
        if self.overlay == Overlay::Compose {
            root=root.child(deferred(div().id("compose-backdrop").absolute().inset_0().flex().items_center().justify_center().bg(rgba(0x102a4690)).occlude()
                .on_scroll(|_,_,cx|cx.stop_propagation())
                .child(panel().id("compose-intent").w(px(640.)).p_6().flex().flex_col().gap_3().occlude()
                    .child(div().text_size(px(22.)).child("Compose intent"))
                    .child("Review the recipient and content supplied by this link. This prototype does not send messages.")
                    .child("To").child(self.compose_to.clone()).child("Subject").child(self.compose_subject.clone()).child("Body").child(self.compose_body.clone())
                    .child(button("compose-close","Close").on_click(cx.listener(|this,_,w,cx|this.set_overlay(Overlay::None,w,cx)))))).with_priority(4));
        }
        if self.overlay == Overlay::Popover {
            root = root.child(deferred(
                panel()
                    .id("popover")
                    .absolute()
                    .top(px(170.))
                    .right(px(36.))
                    .w(px(330.))
                    .p_5()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .occlude()
                    .on_mouse_down_out(
                        cx.listener(|this, _, w, cx| this.set_overlay(Overlay::None, w, cx)),
                    )
                    .child("A GPUI popover over the WPE page. Check the shadow and click handling.")
                    .child(button("popover-modal", "Open modal").on_click(
                        cx.listener(|this, _, w, cx| this.command(Command::Modal, w, cx)),
                    )),
            ));
        }
        root
    }
}
impl Browser {
    fn navigate_click(&mut self, _: &ClickEvent, w: &mut Window, cx: &mut Context<Self>) {
        self.navigate(w, cx);
    }
}
fn email_document(
    mut document: Document,
    prepared: &PreparedEmail,
    dark: bool,
) -> gpui_wpe::Result<Document> {
    let (background, foreground) = if dark {
        ("#17212e", "#e4edf7")
    } else {
        ("#ffffff", "#20364a")
    };
    // App-owned reading-surface defaults; explicit sender colors/images stay intact.
    let html = format!(
        "{}<style>html,body{{background:{background};color:{foreground};margin:0;}}body{{padding:24px;box-sizing:border-box;}}</style>",
        prepared.html
    );
    document = document.with_html(html);
    for (key, resource) in &prepared.resources {
        document = document.with_resource(key.clone(), resource.bytes.clone(), &resource.mime)?;
    }
    Ok(document)
}
fn demo_document(uri: &str) -> Option<Document> {
    match uri {
        "demo://runtime/" | "demo://runtime-soak/" => {
            Some(Document::html(runtime_benchmark::DOCUMENT))
        }
        "demo://editing/" => Some(Document::html(include_str!(
            "../../../fixtures/editing.html"
        ))),
        "demo://second/" => Some(Document::html(include_str!(
            "../../../fixtures/second.html"
        ))),
        "demo://resize/" => Some(Document::html(include_str!(
            "../../../fixtures/resize.html"
        ))),
        _ => None,
    }
}
fn configuration_for_uri(uri: &str) -> ViewConfiguration {
    let restricted = matches!(uri, "demo://resize/" | "demo://runtime-soak/");
    ViewConfiguration {
        settings: WebViewSettings {
            javascript: if restricted {
                JavaScript::Disabled
            } else {
                JavaScript::Enabled
            },
            ..Default::default()
        },
        resources: if restricted {
            ResourcePolicy::ProvidedOnly
        } else {
            ResourcePolicy::Unrestricted
        },
    }
}
fn load_tab(tab: &mut Tab, uri: &str, cx: &mut Context<Browser>) -> gpui_wpe::Result<()> {
    if let Some(document) = demo_document(uri) {
        tab.view.read(cx).load_document(document.clone())?;
        tab.documents.insert(document.id(), document);
        while tab.documents.len() > 8 {
            tab.documents.pop_first();
        }
    } else {
        tab.view.update(cx, |view, _| view.try_load_url(uri))?;
    }
    tab.target_uri = uri.into();
    Ok(())
}
fn compose_intent(uri: &str) -> Option<(String, String, String)> {
    let parsed = url::Url::parse(uri).ok()?;
    if parsed.scheme() != "mailto" {
        return None;
    }
    let bounded = |value: &str| value.chars().take(4096).collect::<String>();
    let mut subject = String::new();
    let mut body = String::new();
    for (key, value) in parsed.query_pairs() {
        match key.as_ref() {
            "subject" => subject = bounded(&value),
            "body" => body = bounded(&value),
            _ => {}
        }
    }
    let recipient = percent_encoding::percent_decode_str(parsed.path())
        .decode_utf8()
        .ok()?;
    if recipient.chars().any(char::is_control) {
        return None;
    }
    Some((
        bounded(&recipient),
        subject.replace(['\r', '\n'], " "),
        body,
    ))
}
fn normalize_uri(input: &str) -> Result<String, &'static str> {
    let text = input.trim();
    if text.is_empty() {
        return Err("Enter a URL");
    }
    if text.starts_with("http://")
        || text.starts_with("https://")
        || matches!(
            text,
            "demo://editing/"
                | "demo://second/"
                | "demo://resize/"
                | "demo://runtime/"
                | "demo://runtime-soak/"
        )
    {
        return Ok(text.into());
    }
    if text.contains(":") || text.chars().any(char::is_whitespace) {
        return Err("Use an HTTP(S) URL or a built-in demo page");
    }
    Ok(format!("https://{text}"))
}
fn main() {
    env_logger::init();
    gpui_platform::linux(WindowingModes::WAYLAND).run(|cx: &mut App| {
        text_input::bind_keys(cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1250.), px(860.)),
                    cx,
                ))),
                ..Default::default()
            },
            |window, cx| {
                window.on_window_should_close(cx, |_, cx| {
                    cx.quit();
                    true
                });
                cx.new(|cx| Browser::new(window, cx))
            },
        )
        .expect("open browser-demo window");
        cx.activate(true);
    });
    if std::env::args().any(|arg| arg == "--benchmark-runtime")
        && !runtime_benchmark::COMPLETED.load(std::sync::atomic::Ordering::Relaxed)
    {
        eprintln!("RUNTIME_BENCHMARK interrupted before final report");
        self_test::FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    if self_test::FAILED.load(std::sync::atomic::Ordering::Relaxed) {
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::{compose_intent, normalize_uri};
    #[test]
    fn mailto_is_an_app_intent() {
        assert_eq!(
            compose_intent(
                "mailto:reader@example.test?subject=Hello%20there&body=Line%201%0ALine%202"
            ),
            Some((
                "reader@example.test".into(),
                "Hello there".into(),
                "Line 1\nLine 2".into()
            ))
        );
        assert!(compose_intent("https://example.test/").is_none());
        assert!(compose_intent("mailto:reader%0A@example.test").is_none());
        assert_eq!(
            compose_intent("mailto:reader%2Btag%40example.test"),
            Some((
                "reader+tag@example.test".into(),
                String::new(),
                String::new()
            ))
        );
    }
    #[test]
    fn address_policy() {
        assert_eq!(
            normalize_uri(" example.com "),
            Ok("https://example.com".into())
        );
        assert_eq!(
            normalize_uri("http://localhost:8080/a"),
            Ok("http://localhost:8080/a".into())
        );
        for uri in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "data:text/html,hi",
            "demo://unknown/",
            "",
            "hello world",
        ] {
            assert!(normalize_uri(uri).is_err(), "{uri}");
        }
    }
}
