//! UI-thread-only ownership of the native WPEPlatform adapter.
use anyhow::{Result, anyhow};
use std::{
    cell::Cell,
    ffi::{CStr, CString, c_char, c_int, c_void},
    io,
    marker::PhantomData,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    ptr::NonNull,
    rc::Rc,
    time::{Duration, Instant},
};

unsafe extern "C" {
    fn bw_hit_test(v: *mut c_void, result: *mut NativeHitTest) -> c_int;
    fn bw_take_context_menu(v: *mut c_void, result: *mut NativeHitTest) -> c_int;
    fn bw_editing_state(v: *mut c_void) -> u32;
    fn bw_clipboard_available(v: *mut c_void) -> c_int;
    fn bw_selection_query_supported(v: *mut c_void) -> c_int;
    fn bw_take_failure(v: *mut c_void, result: *mut NativeFailure) -> c_int;
    fn bw_terminate_process(v: *mut c_void);
    fn bw_take_load_event(v: *mut c_void, result: *mut NativeLoadEvent) -> c_int;
    fn bw_request_selection_text(v: *mut c_void) -> c_int;
    fn bw_take_selection_text(v: *mut c_void, status: *mut c_int) -> *mut c_char;
    fn bw_evaluate(v: *mut c_void, script: *const c_char);
    fn bw_take_evaluation(v: *mut c_void) -> *mut c_char;
    fn bw_create_config(
        display: *mut c_void,
        surface: *mut c_void,
        config: *const NativeConfig,
        error: *mut *mut c_char,
    ) -> *mut c_void;
    fn bw_poll_prepare(fds: *mut NativePollFd, capacity: c_int, timeout_ms: *mut c_int) -> c_int;
    fn bw_wake();
    fn bw_load_document_resources(
        v: *mut c_void,
        id: u64,
        html: *const c_char,
        resources: *const NativeResource,
        count: usize,
    ) -> c_int;
    fn bw_set_resource(
        v: *mut c_void,
        id: u64,
        key: *const c_char,
        bytes: *const u8,
        length: usize,
        mime: *const c_char,
    ) -> c_int;
    fn bw_fail_resource(v: *mut c_void, id: u64, key: *const c_char);
    fn bw_take_resource_request(v: *mut c_void) -> *mut c_char;
    fn bw_take_navigation(v: *mut c_void, flags: *mut u32, token: *mut u64) -> *mut c_char;
    fn bw_resolve_navigation(v: *mut c_void, token: u64, allow: c_int);
    fn bw_find(v: *mut c_void, text: *const c_char, case_sensitive: c_int);
    fn bw_find_next(v: *mut c_void, backwards: c_int);
    fn bw_find_clear(v: *mut c_void);
    fn bw_find_matches(v: *mut c_void) -> u32;
    fn bw_find_pending(v: *mut c_void) -> c_int;
    fn bw_sample_pixel(v: *mut c_void, x: c_int, y: c_int, rgba: *mut u8) -> c_int;
    fn bw_monotonic_us() -> i64;
    fn bw_metrics(v: *mut c_void, metrics: *mut Metrics);
    fn bw_live_metrics(metrics: *mut LiveMetrics);
    fn bw_runtime_info() -> *mut c_char;
    fn bw_get_zoom(v: *mut c_void) -> f64;
    fn bw_destroy(view: *mut c_void);
    fn bw_free(p: *mut c_void);
    fn bw_pump();
    fn bw_shutdown() -> c_int;
    fn bw_geometry(v: *mut c_void, w: c_int, h: c_int, scale: f64, visible: c_int);
    fn bw_clip(v: *mut c_void, x: f64, y: f64, width: f64, height: f64);
    fn bw_focus(v: *mut c_void, focused: c_int);
    fn bw_load(v: *mut c_void, uri: *const c_char);
    fn bw_action(v: *mut c_void, action: c_int);
    fn bw_zoom(v: *mut c_void, zoom: f64);
    fn bw_uri(v: *mut c_void) -> *const c_char;
    fn bw_title(v: *mut c_void) -> *const c_char;
    fn bw_error(v: *mut c_void) -> *const c_char;
    fn bw_cursor(v: *mut c_void) -> *const c_char;
    fn bw_take_new_uri(v: *mut c_void) -> *mut c_char;
    fn bw_state(v: *mut c_void) -> c_int;
    fn bw_frames(v: *mut c_void) -> u64;
    fn bw_document_id(v: *mut c_void) -> u64;
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

#[derive(Clone, Debug)]
pub struct Config {
    pub javascript: bool,
    pub restricted_resources: bool,
    pub mediate_navigation: bool,
    /// Reserved color preference. Only 0 (engine default) is supported by WPE.
    pub theme: i32,
    pub background_rgba: [f64; 4],
}
impl Default for Config {
    fn default() -> Self {
        Self {
            javascript: true,
            restricted_resources: false,
            mediate_navigation: false,
            theme: 0,
            background_rgba: [1., 1., 1., 1.],
        }
    }
}
#[repr(C)]
struct NativeConfig {
    javascript: c_int,
    restricted_resources: c_int,
    mediate_navigation: c_int,
    theme: c_int,
    background: [f64; 4],
}
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
struct NativePollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Metrics {
    pub geometry_generation: u64,
    pub received_frames: u64,
    pub submitted_frames: u64,
    pub discarded_frames: u64,
    pub import_hits: u64,
    pub import_misses: u64,
    pub cached_imports: u64,
    pub outstanding_frames: u64,
    pub geometry_us: i64,
    pub frame_received_us: i64,
    pub acquire_ready_us: i64,
    pub submitted_us: i64,
    pub submitted_generation: u64,
    pub width: c_int,
    pub height: c_int,
}
#[derive(Clone, Copy, Debug)]
pub struct DocumentResource<'a> {
    pub key: &'a str,
    pub bytes: &'a [u8],
    pub mime: &'a str,
}
#[repr(C)]
struct NativeResource {
    key: *const c_char,
    bytes: *const u8,
    length: usize,
    mime: *const c_char,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct LiveMetrics {
    pub views: u64,
    pub displays: u64,
    pub imports: u64,
    pub pending_frames: u64,
    pub sources: u64,
    pub resource_waits: u64,
    pub document_bytes: u64,
    pub resource_bytes: u64,
    /// Shared constant filter compiler/cache jobs retained by this engine.
    pub policy_jobs: u64,
    /// Restricted views waiting for the shared transport blocker.
    pub policy_waiters: u64,
}
/// Exact counts for this bridge's objects and explicit GLib sources; excludes
/// WebKit/driver/compositor internals. Payload byte counts are not allocator
/// usage. The shared display remains retained until explicit shutdown.
/// # Safety
/// Call on the UI thread that owns every live native view.
pub unsafe fn live_metrics() -> LiveMetrics {
    let mut metrics = LiveMetrics::default();
    unsafe { bw_live_metrics(&mut metrics) };
    metrics
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResourceRequest {
    pub document_id: u64,
    pub key: String,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Navigation {
    pub uri: String,
    pub user_gesture: bool,
    pub new_window: bool,
    pub token: u64,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindState {
    pub matches: u32,
    pub pending: bool,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeInfo {
    pub version: String,
    pub library_path: String,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditingState {
    pub can_copy: bool,
    pub can_cut: bool,
    pub can_paste: bool,
    pub can_undo: bool,
    pub can_redo: bool,
}
impl EditingState {
    fn from_flags(flags: u32) -> Self {
        Self {
            can_copy: flags & 1 != 0,
            can_cut: flags & 2 != 0,
            can_paste: flags & 4 != 0,
            can_undo: flags & 8 != 0,
            can_redo: flags & 16 != 0,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HitTest {
    pub link_uri: String,
    pub image_uri: String,
    pub media_uri: String,
    pub link_title: String,
    pub editable: bool,
    pub selection: bool,
    pub scrollbar: bool,
    pub x: i32,
    pub y: i32,
    pub editing: EditingState,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FailureKind {
    Load,
    ProcessTerminated,
    Renderer,
    Clipboard,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub kind: FailureKind,
    pub code: i32,
    pub message: String,
    pub uri: String,
}
#[repr(C)]
#[derive(Default)]
struct NativeHitTest {
    flags: u32,
    editing_flags: u32,
    x: c_int,
    y: c_int,
    link_uri: *const c_char,
    image_uri: *const c_char,
    media_uri: *const c_char,
    link_title: *const c_char,
}
impl NativeHitTest {
    unsafe fn into_hit_test(self) -> HitTest {
        HitTest {
            link_uri: unsafe { string(self.link_uri) },
            image_uri: unsafe { string(self.image_uri) },
            media_uri: unsafe { string(self.media_uri) },
            link_title: unsafe { string(self.link_title) },
            editable: self.flags & 8 != 0,
            selection: self.flags & 16 != 0,
            scrollbar: self.flags & 32 != 0,
            x: self.x,
            y: self.y,
            editing: EditingState::from_flags(self.editing_flags),
        }
    }
}
#[repr(C)]
#[derive(Default)]
struct NativeFailure {
    kind: c_int,
    code: c_int,
    message: *mut c_char,
    uri: *mut c_char,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadPhase {
    Started,
    Redirected,
    Committed,
    Finished,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadEvent {
    pub phase: LoadPhase,
    pub uri: String,
}
#[repr(C)]
#[derive(Default)]
struct NativeLoadEvent {
    phase: c_int,
    uri: *mut c_char,
}
/// Owns duplicated descriptor references so source cancellation cannot cause an
/// unrelated, reused FD to be polled. No GLib/WebKit object crosses threads.
pub struct PollSnapshot {
    fds: Vec<(OwnedFd, i16)>,
    timeout: Option<Duration>,
}
impl PollSnapshot {
    /// Wait on a background executor. This never dispatches GLib or native work.
    pub fn wait(self) -> io::Result<()> {
        let mut fds: Vec<libc::pollfd> = self
            .fds
            .iter()
            .map(|(fd, events)| libc::pollfd {
                fd: fd.as_raw_fd(),
                events: *events,
                revents: 0,
            })
            .collect();
        let started = Instant::now();
        loop {
            let timeout = self
                .timeout
                .map(|limit| {
                    limit
                        .saturating_sub(started.elapsed())
                        .as_millis()
                        .min(c_int::MAX as u128) as c_int
                })
                .unwrap_or(-1);
            // SAFETY: poll has exclusive access to a valid descriptor array;
            // OwnedFd keeps every duplicated open file description alive.
            let result =
                unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout) };
            if result >= 0 {
                return Ok(());
            }
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    }
    pub fn timeout(&self) -> Option<Duration> {
        self.timeout
    }
}
/// # Safety
/// Snapshot GLib sources only on the UI thread owning all live views. The
/// resulting owned snapshot can be moved to a background executor for waiting.
pub unsafe fn prepare_poll() -> io::Result<PollSnapshot> {
    let mut entries = vec![NativePollFd::default(); 32];
    let mut timeout = -1;
    loop {
        let count = unsafe {
            bw_poll_prepare(
                entries.as_mut_ptr(),
                entries.len().min(c_int::MAX as usize) as c_int,
                &mut timeout,
            )
        };
        if count < 0 {
            return Err(io::Error::other("GLib context is owned by another thread"));
        }
        let count = count as usize;
        if count <= entries.len() {
            entries.truncate(count);
            break;
        }
        entries.resize(count, NativePollFd::default());
    }
    let mut fds = vec![];
    for entry in entries {
        if entry.fd < 0 {
            continue;
        }
        // SAFETY: fcntl duplicates a numeric descriptor; on success only this
        // OwnedFd owns the new descriptor. Sources retain their originals.
        let duplicate = unsafe { libc::fcntl(entry.fd, libc::F_DUPFD_CLOEXEC, 0) };
        if duplicate < 0 {
            return Err(io::Error::last_os_error());
        }
        fds.push((unsafe { OwnedFd::from_raw_fd(duplicate) }, entry.events));
    }
    Ok(PollSnapshot {
        fds,
        timeout: (timeout >= 0).then(|| Duration::from_millis(timeout as u64)),
    })
}
/// GLib documents context wakeup as thread-safe. It dispatches nothing itself.
pub fn wake() {
    unsafe { bw_wake() }
}
pub fn runtime_info() -> RuntimeInfo {
    let value = unsafe { take(bw_runtime_info()) }.unwrap_or_default();
    let (version, path) = value.split_once('\n').unwrap_or((&value, "unknown"));
    RuntimeInfo {
        version: version.into(),
        library_path: path.into(),
    }
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
        unsafe { Self::new_with_config(display, surface, &Config::default()) }
    }
    /// # Safety
    /// Same borrowed UI-thread Wayland lifetime requirements as `new`.
    pub unsafe fn new_with_config(
        display: *mut c_void,
        surface: *mut c_void,
        config: &Config,
    ) -> Result<Self> {
        if config
            .background_rgba
            .iter()
            .any(|v| !v.is_finite() || *v < 0. || *v > 1.)
        {
            return Err(anyhow!("Background components must be in [0,1]"));
        }
        if config.theme != 0 {
            return Err(anyhow!(
                "Per-view color-scheme overrides are unsupported by WPE"
            ));
        }
        if config.restricted_resources && config.javascript {
            return Err(anyhow!(
                "Provided-resource-only configuration requires JavaScript disabled"
            ));
        }
        let native = NativeConfig {
            javascript: config.javascript.into(),
            restricted_resources: config.restricted_resources.into(),
            mediate_navigation: config.mediate_navigation.into(),
            theme: config.theme,
            background: config.background_rgba,
        };
        let mut error = std::ptr::null_mut();
        let raw = unsafe { bw_create_config(display, surface, &native, &mut error) };
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
    pub fn set_clip(&self, x: f64, y: f64, width: f64, height: f64) {
        unsafe { bw_clip(self.ptr(), x, y, width, height) }
    }
    /// Query text without page JavaScript or changing the host clipboard.
    /// Requires an engine session that has only hosted scripting-disabled views.
    /// WebKit uses a single primary clipboard; one query may be outstanding.
    pub fn request_selection_text(&self) -> Result<()> {
        match unsafe { bw_request_selection_text(self.ptr()) } {
            1 => Ok(()),
            -1 => Err(anyhow!(
                "Selection text query requires an exclusively scripting-disabled engine session"
            )),
            _ => Err(anyhow!("Selection query busy or clipboard unavailable")),
        }
    }
    pub fn take_selection_text(&self) -> Option<Result<String>> {
        let mut status = 0;
        let text = unsafe { take(bw_take_selection_text(self.ptr(), &mut status)) }?;
        Some(if status > 0 {
            Ok(text)
        } else {
            Err(anyhow!(text))
        })
    }
    pub fn hit_test(&self) -> Option<HitTest> {
        let mut result = NativeHitTest::default();
        (unsafe { bw_hit_test(self.ptr(), &mut result) } != 0)
            .then(|| unsafe { result.into_hit_test() })
    }
    pub fn take_context_menu(&self) -> Option<HitTest> {
        let mut result = NativeHitTest::default();
        (unsafe { bw_take_context_menu(self.ptr(), &mut result) } != 0)
            .then(|| unsafe { result.into_hit_test() })
    }
    pub fn selection_query_supported(&self) -> bool {
        unsafe { bw_selection_query_supported(self.ptr()) != 0 }
    }
    pub fn clipboard_available(&self) -> bool {
        unsafe { bw_clipboard_available(self.ptr()) != 0 }
    }
    pub fn editing_state(&self) -> EditingState {
        EditingState::from_flags(unsafe { bw_editing_state(self.ptr()) })
    }
    pub fn take_load_event(&self) -> Option<LoadEvent> {
        let mut result = NativeLoadEvent::default();
        if unsafe { bw_take_load_event(self.ptr(), &mut result) } == 0 {
            return None;
        }
        Some(LoadEvent {
            phase: match result.phase {
                0 => LoadPhase::Started,
                1 => LoadPhase::Redirected,
                2 => LoadPhase::Committed,
                _ => LoadPhase::Finished,
            },
            uri: unsafe { take(result.uri) }.unwrap_or_default(),
        })
    }
    pub fn take_failure(&self) -> Option<Failure> {
        let mut result = NativeFailure::default();
        if unsafe { bw_take_failure(self.ptr(), &mut result) } == 0 {
            return None;
        }
        Some(Failure {
            kind: match result.kind {
                1 => FailureKind::Load,
                2 => FailureKind::ProcessTerminated,
                4 => FailureKind::Clipboard,
                _ => FailureKind::Renderer,
            },
            code: result.code,
            message: unsafe { take(result.message) }.unwrap_or_default(),
            uri: unsafe { take(result.uri) }.unwrap_or_default(),
        })
    }
    /// Diagnostic process termination for recovery tests.
    pub fn terminate_process(&self) {
        unsafe { bw_terminate_process(self.ptr()) }
    }
    pub fn focus(&self, focused: bool) {
        unsafe { bw_focus(self.ptr(), focused.into()) }
    }
    pub fn document_id(&self) -> u64 {
        unsafe { bw_document_id(self.ptr()) }
    }
    pub fn load_uri(&self, uri: &str) {
        unsafe { bw_load(self.ptr(), cstring(uri).as_ptr()) }
    }
    pub fn load_document(&self, id: u64, html: &str) -> Result<()> {
        self.load_document_with_resources(id, html, &[])
    }
    pub fn load_document_with_resources(
        &self,
        id: u64,
        html: &str,
        resources: &[DocumentResource<'_>],
    ) -> Result<()> {
        if id == 0 || html.len() > 8 * 1024 * 1024 || html.contains('\0') || resources.len() > 1024
        {
            return Err(anyhow!("Invalid or oversized document"));
        }
        let mut names = Vec::with_capacity(resources.len());
        for resource in resources {
            if !valid_resource_key(resource.key)
                || resource.bytes.len() > 32 * 1024 * 1024
                || resource.mime.is_empty()
                || resource.mime.len() > 128
            {
                return Err(anyhow!("Invalid document resource"));
            }
            names.push((CString::new(resource.key)?, CString::new(resource.mime)?));
        }
        let native = resources
            .iter()
            .zip(&names)
            .map(|(resource, (key, mime))| NativeResource {
                key: key.as_ptr(),
                bytes: resource.bytes.as_ptr(),
                length: resource.bytes.len(),
                mime: mime.as_ptr(),
            })
            .collect::<Vec<_>>();
        if unsafe {
            bw_load_document_resources(
                self.ptr(),
                id,
                cstring(html).as_ptr(),
                native.as_ptr(),
                native.len(),
            )
        } == 0
        {
            return Err(anyhow!(
                "Native document identity or quota rejected; previous document retained"
            ));
        }
        Ok(())
    }
    pub fn resource_uri(id: u64, key: &str) -> String {
        format!("gpui-resource://{id}/{key}")
    }
    pub fn set_resource(&self, id: u64, key: &str, bytes: &[u8], mime: &str) -> Result<()> {
        if id == 0
            || !valid_resource_key(key)
            || mime.is_empty()
            || mime.len() > 128
            || mime.contains(['\0', '\r', '\n'])
            || bytes.len() > 32 * 1024 * 1024
        {
            return Err(anyhow!("Invalid application resource"));
        }
        if unsafe {
            bw_set_resource(
                self.ptr(),
                id,
                cstring(key).as_ptr(),
                bytes.as_ptr(),
                bytes.len(),
                cstring(mime).as_ptr(),
            )
        } == 0
        {
            return Err(anyhow!("Resource quota or generation rejected"));
        }
        Ok(())
    }
    pub fn fail_resource(&self, id: u64, key: &str) {
        unsafe { bw_fail_resource(self.ptr(), id, cstring(key).as_ptr()) }
    }
    pub fn take_resource_request(&self) -> Option<ResourceRequest> {
        let raw = unsafe { take(bw_take_resource_request(self.ptr())) }?;
        let (id, key) = raw.split_once('/')?;
        Some(ResourceRequest {
            document_id: id.parse().ok()?,
            key: key.into(),
        })
    }
    pub fn take_navigation(&self) -> Option<Navigation> {
        let mut flags = 0;
        let mut token = 0;
        let uri = unsafe { take(bw_take_navigation(self.ptr(), &mut flags, &mut token)) }?;
        Some(Navigation {
            uri,
            user_gesture: flags & 1 != 0,
            new_window: flags & 2 != 0,
            token,
        })
    }
    pub fn resolve_navigation(&self, token: u64, allow: bool) {
        unsafe { bw_resolve_navigation(self.ptr(), token, allow.into()) }
    }
    pub fn find(&self, text: &str, case_sensitive: bool) {
        unsafe { bw_find(self.ptr(), cstring(text).as_ptr(), case_sensitive.into()) }
    }
    pub fn find_next(&self) {
        unsafe { bw_find_next(self.ptr(), 0) }
    }
    pub fn find_previous(&self) {
        unsafe { bw_find_next(self.ptr(), 1) }
    }
    pub fn find_clear(&self) {
        unsafe { bw_find_clear(self.ptr()) }
    }
    pub fn find_state(&self) -> FindState {
        unsafe {
            FindState {
                matches: bw_find_matches(self.ptr()),
                pending: bw_find_pending(self.ptr()) != 0,
            }
        }
    }
    /// Diagnostic readback of the owned snapshot, in top-left device pixels.
    /// May synchronize with the GPU; excluded from the production render path.
    pub fn sample_pixel(&self, x: i32, y: i32) -> Result<[u8; 4]> {
        let mut rgba = [0; 4];
        if unsafe { bw_sample_pixel(self.ptr(), x, y, rgba.as_mut_ptr()) } == 0 {
            return Err(anyhow!("Pixel unavailable"));
        }
        Ok(rgba)
    }
    pub fn metrics(&self) -> Metrics {
        let mut metrics = Metrics::default();
        unsafe { bw_metrics(self.ptr(), &mut metrics) };
        metrics
    }
    pub fn get_zoom(&self) -> f64 {
        unsafe { bw_get_zoom(self.ptr()) }
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

pub fn valid_resource_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 160
        && key != "."
        && key != ".."
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}
/// Monotonic GLib clock shared by native metrics, in microseconds.
pub fn monotonic_us() -> i64 {
    unsafe { bw_monotonic_us() }
}

#[cfg(test)]
mod tests {
    use super::valid_resource_key;
    #[test]
    fn resource_keys_cannot_escape_document_namespace() {
        for key in ["logo.png", "inline_42", "asset-0"] {
            assert!(valid_resource_key(key));
        }
        for key in [
            "",
            ".",
            "..",
            "../secret",
            "%2e%2e",
            "a/b",
            "a?b",
            "a#b",
            "a\0b",
            "é",
        ] {
            assert!(!valid_resource_key(key), "{key:?}");
        }
    }
}
