mod self_test;
mod text_input;
use gpui::prelude::*;
use gpui::*;
use gpui_wpe::{Action, DEFAULT_URI, Status, WebView};
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
    About,
    Modal,
    Popover,
    Submenu,
    OpenExternal,
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
}
struct Tab {
    view: Entity<WebView>,
    status: Status,
    zoom: f64,
    id: usize,
}
struct Browser {
    tabs: Vec<Tab>,
    active: usize,
    next_id: usize,
    url: Entity<TextInput>,
    modal_input: Entity<TextInput>,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    overlay: Overlay,
    submenu: bool,
    menu_index: usize,
    error: Option<String>,
    pending_link: Option<String>,
    started: Instant,
    self_test: Option<self_test::SelfTest>,
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
        ],
        2 => vec![
            ("Zoom in", ZoomIn),
            ("Zoom out", ZoomOut),
            ("Actual size", ZoomReset),
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
            focus: cx.focus_handle(),
            previous_focus: None,
            overlay: Overlay::None,
            submenu: false,
            menu_index: 0,
            error: None,
            pending_link: None,
            started: Instant::now(),
            self_test: std::env::args()
                .any(|a| a == "--self-test")
                .then(self_test::SelfTest::default),
        };
        let initial = std::env::args()
            .skip(1)
            .find(|s| !s.starts_with("--"))
            .unwrap_or_else(|| DEFAULT_URI.into());
        this.add_tab(
            if this.self_test.is_some() {
                "demo://editing/"
            } else {
                &initial
            },
            window,
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| this.tick(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        this
    }
    fn add_tab(&mut self, uri: &str, window: &mut Window, cx: &mut Context<Self>) {
        let result = WebView::new(window, cx).map(|view| cx.new(|_| view));
        match result {
            Ok(view) => {
                if let Some(tab) = self.tabs.get(self.active) {
                    let _ = tab.view.read(cx).set_active(false);
                }
                view.read(cx).load_uri(uri);
                let id = self.next_id;
                self.next_id += 1;
                self.tabs.push(Tab {
                    view,
                    status: Status {
                        uri: uri.into(),
                        ..Default::default()
                    },
                    zoom: 1.,
                    id,
                });
                self.active = self.tabs.len() - 1;
                self.url.update(cx, |input, cx| input.set_text(uri, cx));
                self.error = None;
            }
            Err(error) => self.error = Some(format!("Cannot create WPE view: {error:#}")),
        }
        cx.notify();
    }
    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index >= self.tabs.len() || index == self.active {
            return;
        }
        self.set_overlay(Overlay::None, window, cx);
        let _ = self.tabs[self.active].view.read(cx).set_active(false);
        self.active = index;
        let tab = &self.tabs[index];
        let _ = tab.view.read(cx).set_active(true);
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
            _ => window.focus(&self.focus, cx),
        }
        cx.notify();
    }
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        gpui_wpe::pump();
        let mut changed = false;
        let mut new_uris = vec![];
        let mut external = None;
        for (index, tab) in self.tabs.iter_mut().enumerate() {
            let (status, new_uri) = tab.view.update(cx, |view, cx| {
                view.tick(window, cx);
                (view.status(), view.take_new_uri())
            });
            if status != tab.status {
                if index == self.active
                    && status.uri != tab.status.uri
                    && !self.url.focus_handle(cx).is_focused(window)
                {
                    self.url
                        .update(cx, |input, cx| input.set_text(&status.uri, cx));
                }
                tab.status = status;
                changed = true;
            }
            if let Some(uri) = new_uri {
                if tab.view.read(cx).is_email() {
                    external = Some(uri);
                } else {
                    new_uris.push(uri);
                }
            }
        }
        if let Some(uri) = external {
            self.pending_link = Some(uri);
            self.set_overlay(Overlay::ExternalLink, window, cx);
        }
        for uri in new_uris {
            self.add_tab(&uri, window, cx);
        }
        if changed {
            cx.notify();
        }
        if let Some(mut test) = self.self_test.take() {
            test.tick(self, window, cx);
            self.self_test = Some(test);
        }
        if std::env::var_os("BROWSER_DEMO_SMOKE").is_some()
            && self.self_test.is_none()
            && self.started.elapsed() > Duration::from_secs(12)
        {
            for tab in &self.tabs {
                eprintln!(
                    "SMOKE uri={} title={:?} frames={} error={:?}",
                    tab.status.uri, tab.status.title, tab.status.frames, tab.status.error
                );
            }
            cx.quit();
        }
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
            tab.view.update(cx, |view, cx| view.action(action, cx));
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
                    self.add_tab(&uri, window, cx);
                }
            }
            Command::NewTab => self.add_tab(DEFAULT_URI, window, cx),
            Command::CloseTab => {
                if !self.tabs.is_empty() {
                    let tab = self.tabs.remove(self.active);
                    tab.view.update(cx, |view, _| {
                        if let Err(e) = view.close(window) {
                            eprintln!("Close tab: {e:#}");
                        }
                    });
                    self.active = self.active.min(self.tabs.len().saturating_sub(1));
                    if self.tabs.is_empty() {
                        self.add_tab(DEFAULT_URI, window, cx);
                    } else {
                        let tab = &self.tabs[self.active];
                        let _ = tab.view.read(cx).set_active(true);
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
                    tab.view.update(cx, |view, cx| match command {
                        Command::Home => view.load_uri(DEFAULT_URI),
                        Command::InputLab => view.load_uri("demo://editing/"),
                        Command::EmailFixture => {
                            view.load_email_html(include_str!("../../../fixtures/email.html"))
                        }
                        Command::Back => view.action(Action::Back, cx),
                        Command::Forward => view.action(Action::Forward, cx),
                        Command::Reload => view.action(Action::Reload, cx),
                        Command::Stop => view.action(Action::Stop, cx),
                        Command::ZoomIn | Command::ZoomOut | Command::ZoomReset => {
                            tab.zoom = match command {
                                Command::ZoomIn => (tab.zoom + 0.1).min(3.),
                                Command::ZoomOut => (tab.zoom - 0.1).max(0.5),
                                _ => 1.,
                            };
                            view.zoom(tab.zoom);
                        }
                        _ => {}
                    });
                }
            }
        }
        cx.notify();
    }
    fn navigate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input = self.url.read(cx).text();
        match normalize_uri(&input) {
            Ok(uri) => {
                if let Some(tab) = self.tabs.get(self.active) {
                    tab.view.read(cx).load_uri(&uri);
                    window.focus(&tab.view.focus_handle(cx), cx);
                    self.error = None;
                }
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
            Overlay::Modal | Overlay::About | Overlay::Popover | Overlay::ExternalLink
        ) {
            return;
        }
        if k.modifiers.control {
            let cmd = match k.key.as_str() {
                "t" => Some(Command::NewTab),
                "w" => Some(Command::CloseTab),
                "r" => Some(Command::Reload),
                "q" => Some(Command::Quit),
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
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
        if let Some(error) = self.error.as_ref().or(if status.error.is_empty() {
            None
        } else {
            Some(&status.error)
        }) {
            root = root.child(div().p_2().bg(rgb(0xffe4d7)).child(error.clone()));
        }
        root=root.child(div().flex_1().min_h(px(0.)).children(self.tabs.get(self.active).map(|t|t.view.clone())))
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
            root=root.child(deferred(div().id("modal-backdrop").absolute().inset_0().flex().items_center().justify_center().bg(rgba(0x102a4690)).occlude()
                .on_scroll(|_,_,cx|cx.stop_propagation())
                .on_mouse_down(MouseButton::Left,cx.listener(|this,_,w,cx|this.set_overlay(Overlay::None,w,cx)))
                .child(panel().id("modal-panel").w(px(480.)).p_6().flex().flex_col().gap_4().occlude()
                    .on_mouse_down(MouseButton::Left,|_,_,cx|cx.stop_propagation())
                    .child(div().text_size(px(23.)).child(if modal{"GPUI overlay test"}else if external{"Open external link?"}else{"About browser-demo"}))
                    .child(if modal{"This dialog, shadow, and translucent backdrop are drawn by GPUI above WPE. Type or paste below; the page underneath must receive no input."}else{"A WPEPlatform WebKit view inside gpui-fast. All browser chrome, tabs, menus and dialogs are GPUI."})
                    .when(modal,|s|s.child(self.modal_input.clone()))
                    .when(external,|s|s.child(self.pending_link.clone().unwrap_or_default()).child(button("open-external","Open in browser tab").on_click(cx.listener(|this,_,w,cx|this.command(Command::OpenExternal,w,cx)))))
                    .child(button("close-modal","Close · Escape").on_click(cx.listener(|this,_,w,cx|this.set_overlay(Overlay::None,w,cx)))))).with_priority(3));
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
fn normalize_uri(input: &str) -> Result<String, &'static str> {
    let text = input.trim();
    if text.is_empty() {
        return Err("Enter a URL");
    }
    if text.starts_with("http://")
        || text.starts_with("https://")
        || matches!(text, "demo://editing/" | "demo://second/")
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
        let browser = cx
            .open_window(
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
        let root = browser
            .update(cx, |_, _, cx| cx.entity())
            .expect("browser entity");
        cx.on_app_quit(move |cx| {
            let _ = browser.update(cx, |browser, window, cx| {
                for tab in browser.tabs.drain(..) {
                    tab.view.update(cx, |view, _| {
                        if let Err(error) = view.close(window) {
                            eprintln!("WPE close: {error:#}");
                        }
                    });
                }
            });
            root.update(cx, |browser, cx| {
                for tab in browser.tabs.drain(..) {
                    tab.view
                        .update(cx, |view, _| view.dispose_after_window_close());
                }
            });
            assert!(
                gpui_wpe::shutdown(),
                "WPE views remained during GPUI shutdown"
            );
            async {}
        })
        .detach();
        cx.activate(true);
    });
    if self_test::FAILED.load(std::sync::atomic::Ordering::Relaxed) {
        std::process::exit(1);
    }
}
#[cfg(test)]
mod tests {
    use super::normalize_uri;
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
