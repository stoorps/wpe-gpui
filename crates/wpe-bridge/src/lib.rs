//! UI-thread-only ownership of the native WPEPlatform adapter.
use anyhow::{Result, anyhow};
use std::{
    cell::Cell,
    ffi::{CStr, CString, c_char, c_int, c_void},
    marker::PhantomData,
    ptr::NonNull,
    rc::Rc,
};

unsafe extern "C" {
    fn bw_evaluate(v: *mut c_void, script: *const c_char);
    fn bw_take_evaluation(v: *mut c_void) -> *mut c_char;
    fn bw_create(
        display: *mut c_void,
        surface: *mut c_void,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn bw_destroy(view: *mut c_void);
    fn bw_free(p: *mut c_void);
    fn bw_pump();
    fn bw_shutdown() -> c_int;
    fn bw_geometry(v: *mut c_void, w: c_int, h: c_int, scale: f64, visible: c_int);
    fn bw_focus(v: *mut c_void, focused: c_int);
    fn bw_load(v: *mut c_void, uri: *const c_char);
    fn bw_load_html(v: *mut c_void, html: *const c_char, base: *const c_char, email: c_int);
    fn bw_action(v: *mut c_void, action: c_int);
    fn bw_zoom(v: *mut c_void, zoom: f64);
    fn bw_uri(v: *mut c_void) -> *const c_char;
    fn bw_title(v: *mut c_void) -> *const c_char;
    fn bw_error(v: *mut c_void) -> *const c_char;
    fn bw_cursor(v: *mut c_void) -> *const c_char;
    fn bw_take_new_uri(v: *mut c_void) -> *mut c_char;
    fn bw_state(v: *mut c_void) -> c_int;
    fn bw_frames(v: *mut c_void) -> u64;
    fn bw_pointer(
        v: *mut c_void,
        kind: c_int,
        x: f64,
        y: f64,
        button: c_int,
        mods: u32,
        count: c_int,
    );
    fn bw_scroll(v: *mut c_void, x: f64, y: f64, dx: f64, dy: f64, mods: u32);
    fn bw_key(v: *mut c_void, key: *const c_char, mods: u32, down: c_int);
    fn bw_text(v: *mut c_void, text: *const c_char, preedit: c_int);
    fn bw_set_clipboard(v: *mut c_void, text: *const c_char);
    fn bw_take_clipboard(v: *mut c_void) -> *mut c_char;
}

#[derive(Clone, Copy, Debug)]
#[repr(i32)]
pub enum Action {
    Back,
    Forward,
    Reload,
    Stop,
    Copy,
    Cut,
    Paste,
    SelectAll,
    Undo,
    Redo,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    pub uri: String,
    pub title: String,
    pub error: String,
    pub cursor: String,
    pub loading: bool,
    pub can_back: bool,
    pub can_forward: bool,
    pub frames: u64,
}
/// Not Send/Sync: every method and final drop must run on its creating UI thread.
pub struct View {
    raw: Cell<Option<NonNull<c_void>>>,
    _ui_thread: PhantomData<Rc<()>>,
}
fn cstring(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap()
}
unsafe fn string(p: *const c_char) -> String {
    if p.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }
}
unsafe fn take(p: *mut c_char) -> Option<String> {
    if p.is_null() {
        None
    } else {
        let value = unsafe { string(p) };
        unsafe { bw_free(p.cast()) };
        Some(value)
    }
}
impl View {
    /// # Safety
    /// `display` and `surface` must be live Wayland handles owned by GPUI on
    /// this thread. The surface must remain live until this view is closed.
    /// The borrowed display must also remain live until this view is closed.
    pub unsafe fn new(display: *mut c_void, surface: *mut c_void) -> Result<Self> {
        let mut error = std::ptr::null_mut();
        let raw = unsafe { bw_create(display, surface, &mut error) };
        let message = unsafe { take(error) };
        Ok(Self {
            raw: Cell::new(Some(NonNull::new(raw).ok_or_else(|| {
                anyhow!(message.unwrap_or_else(|| "Cannot create WPE view".into()))
            })?)),
            _ui_thread: PhantomData,
        })
    }
    /// Stop native presentation immediately, even if GPUI retains an old frame.
    pub fn close(&self) {
        if let Some(raw) = self.raw.take() {
            unsafe { bw_destroy(raw.as_ptr()) }
        }
    }
    fn ptr(&self) -> *mut c_void {
        self.raw.get().expect("WPE view is closed").as_ptr()
    }
    pub fn geometry(&self, width: i32, height: i32, scale: f64, visible: bool) {
        if !scale.is_finite() || scale <= 0. {
            return;
        }
        unsafe {
            bw_geometry(
                self.ptr(),
                width.max(0),
                height.max(0),
                scale,
                visible.into(),
            )
        }
    }
    pub fn focus(&self, focused: bool) {
        unsafe { bw_focus(self.ptr(), focused.into()) }
    }
    pub fn load_uri(&self, uri: &str) {
        unsafe { bw_load(self.ptr(), cstring(uri).as_ptr()) }
    }
    /// Loads trusted application HTML. Untrusted email requires sanitization and
    /// enforceable resource policy before calling this; see findings.md.
    pub fn load_trusted_html(&self, html: &str, base: &str, email: bool) {
        unsafe {
            bw_load_html(
                self.ptr(),
                cstring(html).as_ptr(),
                cstring(base).as_ptr(),
                email.into(),
            )
        }
    }
    pub fn action(&self, action: Action) {
        unsafe { bw_action(self.ptr(), action as i32) }
    }
    pub fn zoom(&self, zoom: f64) {
        unsafe { bw_zoom(self.ptr(), zoom.clamp(0.5, 3.)) }
    }
    pub fn status(&self) -> Status {
        unsafe {
            let flags = bw_state(self.ptr());
            Status {
                uri: string(bw_uri(self.ptr())),
                title: string(bw_title(self.ptr())),
                error: string(bw_error(self.ptr())),
                cursor: string(bw_cursor(self.ptr())),
                loading: flags & 1 != 0,
                can_back: flags & 2 != 0,
                can_forward: flags & 4 != 0,
                frames: bw_frames(self.ptr()),
            }
        }
    }
    pub fn evaluate(&self, script: &str) {
        unsafe { bw_evaluate(self.ptr(), cstring(script).as_ptr()) }
    }
    pub fn take_evaluation(&self) -> Option<String> {
        unsafe { take(bw_take_evaluation(self.ptr())) }
    }
    pub fn take_new_uri(&self) -> Option<String> {
        unsafe { take(bw_take_new_uri(self.ptr())) }
    }
    pub fn pointer(&self, kind: i32, x: f64, y: f64, button: i32, modifiers: u32, count: i32) {
        unsafe { bw_pointer(self.ptr(), kind, x, y, button, modifiers, count) }
    }
    pub fn scroll(&self, x: f64, y: f64, dx: f64, dy: f64, modifiers: u32) {
        unsafe { bw_scroll(self.ptr(), x, y, dx, dy, modifiers) }
    }
    pub fn key(&self, key: &str, modifiers: u32, down: bool) {
        unsafe { bw_key(self.ptr(), cstring(key).as_ptr(), modifiers, down.into()) }
    }
    pub fn text(&self, text: &str, preedit: bool) {
        unsafe { bw_text(self.ptr(), cstring(text).as_ptr(), preedit.into()) }
    }
    pub fn set_clipboard(&self, text: &str) {
        unsafe { bw_set_clipboard(self.ptr(), cstring(text).as_ptr()) }
    }
    pub fn take_clipboard(&self) -> Option<String> {
        unsafe { take(bw_take_clipboard(self.ptr())) }
    }
}
impl Drop for View {
    fn drop(&mut self) {
        self.close()
    }
}
/// Drive a bounded number of nonblocking GLib iterations on the UI thread.
/// Kept unsafe so unrelated worker threads cannot accidentally dispatch WebKit.
/// # Safety
/// Call only from the same UI thread that owns all live `View`s.
pub unsafe fn pump() {
    unsafe { bw_pump() }
}

/// Release the shared WPE display after all GPUI windows and renderers drop.
/// # Safety
/// Call on the owning UI thread before the borrowed Wayland connection drops.
/// Close views and call synchronously in `on_app_quit`, before GPUI clears windows.
pub unsafe fn shutdown() -> bool {
    unsafe { bw_shutdown() != 0 }
}
