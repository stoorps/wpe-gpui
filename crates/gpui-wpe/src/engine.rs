use crate::{DocumentId, Error, Native, ResourceResponse, Result, resource};
use gpui::*;
use std::{
    cell::RefCell,
    rc::{Rc, Weak},
    sync::{
        Arc, Mutex,
        atomic::AtomicBool,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
static INSTANCE: AtomicU64 = AtomicU64::new(1);
pub(super) fn next_instance() -> u64 {
    INSTANCE.fetch_add(1, Ordering::Relaxed)
}
const MAX_PENDING_RESPONSES: usize = 128;
const MAX_QUEUED_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const RESOURCE_DEADLINE: Duration = Duration::from_secs(5);

enum Completion {
    Waiting,
    Ready(Option<ResourceResponse>),
    Cancelled,
}
// Keep the byte reservation through UI installation, not just queue removal:
// workers can finish other slots while the UI is processing a delivery batch.
struct ReadyResponse {
    response: Option<ResourceResponse>,
    queued_bytes: Arc<Mutex<usize>>,
    bytes: usize,
}
impl Drop for ReadyResponse {
    fn drop(&mut self) {
        // Drop the allocation before making the budget available to workers.
        self.response.take();
        *self.queued_bytes.lock().unwrap() -= self.bytes;
    }
}
struct ResponseSlot {
    completion: Mutex<Completion>,
    queued_bytes: Arc<Mutex<usize>>,
    liveness: Arc<AtomicBool>,
    deadline: Instant,
}
impl ResponseSlot {
    fn cancelled(&self) -> bool {
        !self.liveness.load(Ordering::Acquire) || Instant::now() >= self.deadline
    }
    fn settle(&self, response: Option<ResourceResponse>) -> Result<()> {
        let mut state = self.completion.lock().unwrap();
        if !matches!(*state, Completion::Waiting) || self.cancelled() {
            return Err(Error::Cancelled);
        }
        let bytes = response.as_ref().map_or(0, |r| r.bytes.len());
        let mut queued = self.queued_bytes.lock().unwrap();
        if bytes > MAX_QUEUED_RESPONSE_BYTES - *queued {
            // A request reserves its failure completion before the application
            // sees it, so overload cannot lose the native error response.
            *state = Completion::Ready(None);
            drop(queued);
            drop(state);
            wpe_bridge::wake();
            return Err(Error::Native(
                "resource response byte budget exceeded".into(),
            ));
        }
        *queued += bytes;
        *state = Completion::Ready(response);
        drop(queued);
        drop(state);
        wpe_bridge::wake();
        Ok(())
    }
    fn take(&self, cancel: bool) -> Option<ReadyResponse> {
        let mut state = self.completion.lock().unwrap();
        if matches!(*state, Completion::Waiting) && !cancel && !self.cancelled() {
            return None;
        }
        let old = std::mem::replace(&mut *state, Completion::Cancelled);
        let response = match old {
            Completion::Ready(response) => response,
            Completion::Waiting | Completion::Cancelled => None,
        };
        let bytes = response.as_ref().map_or(0, |r| r.bytes.len());
        Some(ReadyResponse {
            response: if cancel || self.cancelled() {
                None
            } else {
                response
            },
            queued_bytes: self.queued_bytes.clone(),
            bytes,
        })
    }
}
struct PendingResponse {
    instance: u64,
    epoch: u64,
    document: DocumentId,
    key: String,
    slot: Arc<ResponseSlot>,
}
struct Engine {
    installed: bool,
    running: bool,
    driver_generation: u64,
    shutting_down: bool,
    quitting: bool,
    views: Vec<Weak<Native>>,
    pending: Vec<PendingResponse>,
    queued_bytes: Arc<Mutex<usize>>,
    orphans: Vec<(
        AnyWindowHandle,
        CompositionSurfaceId,
        WindowCompositionSurface,
    )>,
}
impl Engine {
    fn reserve_response(
        &mut self,
        instance: u64,
        epoch: u64,
        document: DocumentId,
        key: String,
        liveness: Arc<AtomicBool>,
    ) -> Option<Arc<ResponseSlot>> {
        if self.pending.len() >= MAX_PENDING_RESPONSES {
            return None;
        }
        let slot = Arc::new(ResponseSlot {
            completion: Mutex::new(Completion::Waiting),
            queued_bytes: self.queued_bytes.clone(),
            liveness,
            deadline: Instant::now() + RESOURCE_DEADLINE,
        });
        self.pending.push(PendingResponse {
            instance,
            epoch,
            document,
            key,
            slot: slot.clone(),
        });
        Some(slot)
    }
}
impl Default for Engine {
    fn default() -> Self {
        Self {
            installed: false,
            running: false,
            driver_generation: 0,
            shutting_down: false,
            quitting: false,
            views: vec![],
            pending: vec![],
            queued_bytes: Arc::new(Mutex::new(0)),
            orphans: vec![],
        }
    }
}
thread_local! {static ENGINE:RefCell<Engine>=RefCell::new(Engine::default());}
pub(super) fn ensure_creation_allowed() -> Result<()> {
    if ENGINE.with(|engine| engine.borrow().quitting) {
        Err(Error::Closed)
    } else {
        Ok(())
    }
}
/// A reserved, bounded response capability which may move to an application
/// worker. Drop fails the resource. No native pointer leaves the UI thread.
pub struct ResourceResponder {
    slot: Arc<ResponseSlot>,
    max_bytes: usize,
}
impl ResourceResponder {
    /// True after replacement, close, the request deadline, or completion.
    pub fn is_cancelled(&self) -> bool {
        self.slot.cancelled()
            || !matches!(*self.slot.completion.lock().unwrap(), Completion::Waiting)
    }
    pub fn respond(self, response: ResourceResponse) -> Result<()> {
        if response.bytes.len() > self.max_bytes {
            return Err(Error::InvalidInput(
                "resource response exceeds limit".into(),
            ));
        }
        if response.mime_type.is_empty()
            || response.mime_type.len() > 128
            || response
                .mime_type
                .bytes()
                .any(|byte| byte < 32 || byte == 127)
        {
            return Err(Error::InvalidInput(
                "invalid resource response MIME type".into(),
            ));
        }
        self.slot.settle(Some(response))
    }
    pub fn fail(self) -> Result<()> {
        self.slot.settle(None)
    }
}
impl Drop for ResourceResponder {
    fn drop(&mut self) {
        let _ = self.slot.settle(None);
    }
}
pub(super) fn register(native: &Rc<Native>, cx: &mut App) {
    let (install, start, generation) = ENGINE.with(|engine| {
        let mut e = engine.borrow_mut();
        e.views.retain(|v| v.strong_count() > 0);
        e.views.push(Rc::downgrade(native));
        let install = !e.installed;
        let start = !e.running;
        e.installed = true;
        e.running = true;
        if start {
            e.driver_generation += 1;
        }
        e.shutting_down = false;
        (install, start, e.driver_generation)
    });
    if install {
        cx.on_window_closed(|cx, id| {
            let views = ENGINE.with(|engine| {
                engine
                    .borrow()
                    .views
                    .iter()
                    .filter_map(Weak::upgrade)
                    .filter(|native| native.window.window_id() == id)
                    .collect::<Vec<_>>()
            });
            for native in views {
                let callback = native.notify.borrow_mut().take();
                native.stop_native();
                native.surface.borrow_mut().take();
                if let Some(callback) = callback {
                    cx.defer(move |cx| callback(cx));
                }
            }
            drain_orphans(cx);
            wpe_bridge::wake();
        })
        .detach();
        cx.on_app_quit(|cx| {
            let views = ENGINE.with(|e| {
                let mut e = e.borrow_mut();
                e.shutting_down = true;
                e.quitting = true;
                e.running = false;
                e.driver_generation += 1;
                e.installed = false;
                for pending in e.pending.drain(..) {
                    pending.slot.take(true);
                }
                e.views.iter().filter_map(Weak::upgrade).collect::<Vec<_>>()
            });
            drain_orphans(cx);
            for native in views {
                close_native(native, cx);
            }
            wpe_bridge::wake();
            // All native presenters and composition references have been released
            // while GPUI's Wayland callback state is still alive.
            if !unsafe { wpe_bridge::shutdown() } {
                eprintln!("WPE shutdown retained native views");
            }
            async {}
        })
        .detach();
    }
    if !start {
        wpe_bridge::wake();
        return;
    }
    cx.spawn(async move |cx| {
        loop {
            if ENGINE.with(|e| {
                let e = e.borrow();
                e.shutting_down || e.driver_generation != generation
            }) {
                break;
            }
            // Snapshot contains only owned descriptor duplicates and a deadline.
            // The worker never dispatches GLib, EGL, or WebKit.
            let snapshot = match unsafe { wpe_bridge::prepare_poll() } {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    cx.update(|cx| engine_failed(error, cx));
                    break;
                }
            };
            let result = cx
                .background_executor()
                .spawn(async move { snapshot.wait() })
                .await;
            if ENGINE.with(|e| {
                let e = e.borrow();
                e.shutting_down || e.driver_generation != generation
            }) {
                break;
            }
            if let Err(error) = result {
                cx.update(|cx| engine_failed(error, cx));
                break;
            }
            cx.update(|cx| {
                if ENGINE.with(|e| e.borrow().shutting_down) {
                    return;
                }
                unsafe { wpe_bridge::pump() };
                service(cx);
            });
        }
        ENGINE.with(|e| {
            let mut e = e.borrow_mut();
            if e.driver_generation == generation {
                e.running = false;
            }
        });
    })
    .detach();
    wpe_bridge::wake();
}
pub(super) fn close_native(native: Rc<Native>, cx: &mut App) {
    let callback = native.notify.borrow_mut().take();
    native.stop_native();
    if let Some(callback) = callback {
        cx.defer(move |cx| callback(cx));
    }
    let handle = native.window;
    // A command may be issued from a borrowed window update. In that case the
    // detached cleanup runs at the end of the effect cycle, never at App drop.
    let n = native.clone();
    let result = handle.update(cx, |_, window, _| {
        if n.surface.borrow().is_some() {
            let _ = window
                .enable_window_composition()
                .and_then(|c| c.remove_surface(n.id));
        }
        n.surface.borrow_mut().take();
    });
    if result.is_err() {
        cx.defer(move |cx| {
            let n = native.clone();
            let _ = handle.update(cx, |_, window, _| {
                let _ = window
                    .enable_window_composition()
                    .and_then(|c| c.remove_surface(n.id));
            });
            native.surface.borrow_mut().take();
        });
    }
    wpe_bridge::wake();
}
pub(super) fn release_surface(
    window: AnyWindowHandle,
    id: CompositionSurfaceId,
    surface: WindowCompositionSurface,
) {
    ENGINE.with(|e| e.borrow_mut().orphans.push((window, id, surface)));
    wpe_bridge::wake();
}
fn drain_orphans(cx: &mut App) {
    let orphans = ENGINE.with(|e| std::mem::take(&mut e.borrow_mut().orphans));
    for (handle, id, surface) in orphans {
        let _ = handle.update(cx, |_, window, _| {
            let _ = window
                .enable_window_composition()
                .and_then(|c| c.remove_surface(id));
        });
        drop(surface);
    }
}
fn deliver_responses(views: &[Rc<Native>]) {
    // Take only completed/cancelled slots. Waiting workers keep their reserved
    // slot and can always deliver failure even when the byte budget is full.
    let jobs = ENGINE.with(|engine| {
        let mut e = engine.borrow_mut();
        let mut jobs = Vec::new();
        e.pending.retain(|pending| {
            let active = views.iter().any(|n| {
                n.instance == pending.instance
                    && !n.closed.get()
                    && n.epoch.get() == pending.epoch
                    && n.document.get() == Some(pending.document)
            });
            if let Some(response) = pending.slot.take(!active) {
                if active {
                    jobs.push((
                        pending.instance,
                        pending.document,
                        pending.key.clone(),
                        response,
                    ));
                }
                false
            } else {
                true
            }
        });
        jobs
    });
    for (instance, document, key, mut ready) in jobs {
        let Some(n) = views.iter().find(|n| n.instance == instance) else {
            continue;
        };
        if let Some(response) = ready.response.take() {
            if let Err(error) = n.install_resource(document, &key, &response) {
                n.view.fail_resource(document.get(), &key);
                n.pending_errors.borrow_mut().push(error);
            }
        } else {
            n.view.fail_resource(document.get(), &key);
        }
    }
}
fn service(cx: &mut App) {
    drain_orphans(cx);
    let views = ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        e.views.retain(|v| v.strong_count() > 0);
        e.views.iter().filter_map(Weak::upgrade).collect::<Vec<_>>()
    });
    for native in &views {
        if !native.closed.get() {
            native.sync_document();
        }
    }
    deliver_responses(&views);
    for native in &views {
        if native.closed.get() {
            continue;
        }
        while let Some(request) = native.view.take_resource_request() {
            let Some(id) = native
                .document
                .get()
                .filter(|id| id.get() == request.document_id)
            else {
                native.view.fail_resource(request.document_id, &request.key);
                continue;
            };
            if resource::validate_key(&request.key).is_err() {
                native.view.fail_resource(request.document_id, &request.key);
                continue;
            }
            if let Some(handler) = &native.resource_handler {
                let slot = ENGINE.with(|engine| {
                    let mut e = engine.borrow_mut();
                    e.reserve_response(
                        native.instance,
                        native.epoch.get(),
                        id,
                        request.key.clone(),
                        native.resource_liveness.borrow().clone(),
                    )
                });
                if let Some(slot) = slot {
                    handler(
                        resource::ResourceRequest {
                            document_id: id,
                            key: request.key,
                        },
                        ResourceResponder {
                            slot,
                            max_bytes: native.settings.max_resource_bytes,
                        },
                    );
                    // Immediate handlers can fill arbitrarily many distinct
                    // resources without filling a completion queue.
                    deliver_responses(&views);
                } else {
                    native.view.fail_resource(id.get(), &request.key);
                }
            } else {
                native.view.fail_resource(request.document_id, &request.key);
            }
        }
        let callback = native.notify.borrow_mut().take();
        if let Some(callback) = callback {
            callback(cx);
            if !native.closed.get() {
                *native.notify.borrow_mut() = Some(callback);
            }
        }
    }
}

fn engine_failed(error: std::io::Error, cx: &mut App) {
    let views = ENGINE.with(|e| {
        let mut e = e.borrow_mut();
        e.shutting_down = true;
        e.running = false;
        e.driver_generation += 1;
        for pending in e.pending.drain(..) {
            pending.slot.take(true);
        }
        e.views.iter().filter_map(Weak::upgrade).collect::<Vec<_>>()
    });
    for native in views {
        native
            .pending_errors
            .borrow_mut()
            .push(Error::Native(format!(
                "event-loop servicing failed: {error}"
            )));
        close_native(native, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Completion, DocumentId, ENGINE, Engine, Error, MAX_PENDING_RESPONSES,
        MAX_QUEUED_RESPONSE_BYTES, RESOURCE_DEADLINE, ResourceResponder, ResourceResponse,
        ResponseSlot, Result, ensure_creation_allowed,
    };
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        time::{Duration, Instant},
    };
    fn slot(budget: Arc<Mutex<usize>>) -> Arc<ResponseSlot> {
        Arc::new(ResponseSlot {
            completion: Mutex::new(Completion::Waiting),
            queued_bytes: budget,
            liveness: Arc::new(AtomicBool::new(true)),
            deadline: Instant::now() + RESOURCE_DEADLINE,
        })
    }
    fn response(bytes: &[u8]) -> ResourceResponse {
        ResourceResponse::new(bytes, "image/png").unwrap()
    }
    #[test]
    fn application_quit_denies_new_views_but_driver_failure_can_restart() {
        ENGINE.with(|engine| engine.borrow_mut().shutting_down = true);
        assert!(ensure_creation_allowed().is_ok());
        ENGINE.with(|engine| engine.borrow_mut().quitting = true);
        assert!(matches!(ensure_creation_allowed(), Err(Error::Closed)));
        ENGINE.with(|engine| *engine.borrow_mut() = Engine::default());
    }
    #[test]
    fn reserved_completions_survive_drop_and_settle_exactly_once() {
        let budget = Arc::new(Mutex::new(0));
        let a = slot(budget.clone());
        ResourceResponder {
            slot: a.clone(),
            max_bytes: 8,
        }
        .respond(response(b"abc"))
        .unwrap();
        assert_eq!(*budget.lock().unwrap(), 3);
        assert_eq!(
            a.take(false).unwrap().response.as_ref().unwrap().bytes,
            b"abc"
        );
        assert_eq!(*budget.lock().unwrap(), 0);
        assert!(a.settle(Some(response(b"again"))).is_err());
        let b = slot(budget);
        drop(ResourceResponder {
            slot: b.clone(),
            max_bytes: 8,
        });
        assert!(b.take(false).unwrap().response.is_none());
    }
    #[test]
    fn overload_fails_a_reserved_request_without_stranding_it() {
        let budget = Arc::new(Mutex::new(MAX_QUEUED_RESPONSE_BYTES - 2));
        let a = slot(budget.clone());
        assert!(a.settle(Some(response(b"abc"))).is_err());
        assert!(a.take(false).unwrap().response.is_none());
        assert_eq!(*budget.lock().unwrap(), MAX_QUEUED_RESPONSE_BYTES - 2);
    }
    #[test]
    fn cancellation_releases_queued_bytes_and_workers_observe_expiry() {
        let budget = Arc::new(Mutex::new(0));
        let a = slot(budget.clone());
        a.settle(Some(response(b"private"))).unwrap();
        a.liveness.store(false, Ordering::Release);
        assert!(a.take(false).unwrap().response.is_none());
        assert_eq!(*budget.lock().unwrap(), 0);
        let expired = Arc::new(ResponseSlot {
            completion: Mutex::new(Completion::Waiting),
            queued_bytes: budget,
            liveness: Arc::new(AtomicBool::new(true)),
            deadline: Instant::now() - Duration::from_secs(1),
        });
        let responder = ResourceResponder {
            slot: expired.clone(),
            max_bytes: 8,
        };
        assert!(responder.is_cancelled());
        assert!(responder.respond(response(b"late")).is_err());
        assert!(expired.take(false).unwrap().response.is_none());
    }
    #[test]
    fn concurrent_workers_cannot_exceed_the_response_byte_budget() {
        let budget = Arc::new(Mutex::new(MAX_QUEUED_RESPONSE_BYTES - 3));
        let a = slot(budget.clone());
        let b = slot(budget.clone());
        let first = a.clone();
        let second = b.clone();
        let x = std::thread::spawn(move || first.settle(Some(response(b"ab"))));
        let y = std::thread::spawn(move || second.settle(Some(response(b"cd"))));
        let accepted = [x.join().unwrap(), y.join().unwrap()]
            .into_iter()
            .filter(Result::is_ok)
            .count();
        assert_eq!(accepted, 1);
        assert_eq!(*budget.lock().unwrap(), MAX_QUEUED_RESPONSE_BYTES - 1);
        a.take(true);
        b.take(true);
        assert_eq!(*budget.lock().unwrap(), MAX_QUEUED_RESPONSE_BYTES - 3);
    }
    #[test]
    fn ui_delivery_holds_budget_until_the_allocation_is_released() {
        let budget = Arc::new(Mutex::new(0));
        let a = slot(budget.clone());
        a.settle(Some(response(b"pixels"))).unwrap();
        let delivery = a.take(false).unwrap();
        assert_eq!(*budget.lock().unwrap(), 6);
        drop(delivery);
        assert_eq!(*budget.lock().unwrap(), 0);
    }
    #[test]
    fn invalid_response_metadata_uses_its_reserved_failure() {
        let a = slot(Arc::new(Mutex::new(0)));
        let invalid = ResourceResponse {
            bytes: vec![],
            mime_type: "a".repeat(129),
        };
        assert!(
            ResourceResponder {
                slot: a.clone(),
                max_bytes: 8
            }
            .respond(invalid)
            .is_err()
        );
        assert!(a.take(false).unwrap().response.is_none());
    }
    #[test]
    fn request_admission_is_bounded_before_handlers_run() {
        let mut engine = Engine::default();
        for _ in 0..MAX_PENDING_RESPONSES {
            assert!(
                engine
                    .reserve_response(
                        1,
                        1,
                        DocumentId::new(),
                        "image".into(),
                        Arc::new(AtomicBool::new(true))
                    )
                    .is_some()
            );
        }
        assert!(
            engine
                .reserve_response(
                    1,
                    1,
                    DocumentId::new(),
                    "extra".into(),
                    Arc::new(AtomicBool::new(true))
                )
                .is_none()
        );
        assert_eq!(engine.pending.len(), MAX_PENDING_RESPONSES);
    }
}
