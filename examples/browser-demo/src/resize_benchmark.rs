//! Diagnostic resize driver. Samples the native owned snapshot, never page JS.
use super::*;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Default)]
pub struct Benchmark {
    started: Option<Instant>,
    continuous: Option<Instant>,
    steps: usize,
    request: Option<Request>,
    samples: Vec<Value>,
    last_resize: Option<Instant>,
    finished: bool,
}
struct Request {
    number: usize,
    baseline: u64,
    issued_us: i64,
    issued: Instant,
    final_sample: bool,
    target_width: f32,
    target_height: f32,
    incorrect_frames: u32,
    last_incorrect_us: i64,
}
impl Benchmark {
    fn issue(
        &mut self,
        b: &Browser,
        w: &mut Window,
        cx: &mut Context<Browser>,
        continuous: bool,
        final_sample: bool,
    ) {
        let Some(tab) = b.tabs.get(b.active) else {
            return;
        };
        let baseline = tab
            .view
            .read(cx)
            .metrics()
            .map(|m| m.geometry_generation)
            .unwrap_or(0);
        let number = self.samples.len() + 1;
        let width = [720., 960., 1120., 1280.][number % 4];
        let issued_us = gpui_wpe::monotonic_us();
        let height = if continuous && number.is_multiple_of(2) {
            660.
        } else {
            740.
        };
        w.resize(size(px(width), px(height)));
        self.request = Some(Request {
            number,
            baseline,
            issued_us,
            issued: Instant::now(),
            final_sample,
            target_width: width,
            target_height: height,
            incorrect_frames: 0,
            last_incorrect_us: 0,
        });
        self.last_resize = Some(Instant::now());
        cx.notify();
    }
    fn capture(&mut self, b: &Browser, w: &Window, cx: &App, superseded: bool) -> bool {
        let Some(request) = self.request.as_mut() else {
            return false;
        };
        let Some(tab) = b.tabs.get(b.active) else {
            return false;
        };
        let view = tab.view.read(cx);
        let Ok(m) = view.metrics() else {
            return false;
        };
        let ready = m.geometry_generation > request.baseline
            && m.submitted_generation == m.geometry_generation
            && m.geometry_us >= request.issued_us
            && m.submitted_us >= m.geometry_us;
        let timed_out = request.issued.elapsed() > Duration::from_millis(500);
        if !ready && !timed_out && !superseded {
            return false;
        }
        let expected: [u8; 3] = if m.width >= 1200 {
            [240, 180, 30]
        } else if m.width >= 1000 {
            [50, 100, 220]
        } else if m.width >= 800 {
            [40, 180, 80]
        } else {
            [220, 40, 40]
        };
        let pixel = if ready && !superseded {
            view.sample_pixel(16, 16).ok()
        } else {
            None
        };
        let marker = pixel
            .is_some_and(|p| (0..3).all(|i| (i16::from(p[i]) - i16::from(expected[i])).abs() <= 4));
        let width_matches =
            (f32::from(view.viewport_bounds().size.width).round() as i32 - m.width).abs() <= 1;
        let correct = ready && marker && width_matches;
        if ready && !correct && m.submitted_us != request.last_incorrect_us {
            request.incorrect_frames += 1;
            request.last_incorrect_us = m.submitted_us;
        }
        if !correct && !timed_out && !superseded {
            return false;
        }
        let t0 = if ready {
            view.geometry_timestamp_us().max(request.issued_us)
        } else {
            request.issued_us
        };
        self.samples.push(json!({
            "generation":self.samples.len()+1,"native_generation":m.geometry_generation,
            "window_active":w.is_window_active(),
            "device_scale":w.scale_factor(),
            "drawable_size":[(m.width as f64*w.scale_factor() as f64).ceil(),(m.height as f64*w.scale_factor() as f64).ceil()],
            "target_size":[request.target_width,request.target_height],
            "window_size":[f32::from(w.bounds().size.width),f32::from(w.bounds().size.height)],
            "gpui_viewport_size":[f32::from(view.viewport_bounds().size.width),f32::from(view.viewport_bounds().size.height)],
            "geometry_change_accepted":m.geometry_generation>request.baseline&&m.geometry_us>=request.issued_us,
            "transient_incorrect_frames":request.incorrect_frames,
            "document_id":view.state().document_id.map(|id|id.get()).unwrap_or(0),
            "fixture":"responsive-email","t0_us":t0,
            "t2_us":if ready{Some(m.geometry_us)}else{None},
            "received_us":if ready{Some(m.frame_received_us)}else{None},
            "acquire_ready_us":if ready{Some(m.acquire_ready_us)}else{None},
            "submitted_us":if ready{Some(m.submitted_us)}else{None},
            "correct":correct,"superseded":superseded,"validation":"fixture-marker",
            "final":request.final_sample,"width":m.width,"height":m.height,
            "marker_rgba":pixel,"marker_expected_rgb":expected,"request_number":request.number,
            "import_hits":m.import_hits,"import_misses":m.import_misses,
            "cached_imports":m.cached_imports,"outstanding_frames":m.outstanding_frames,
        }));
        self.request = None;
        true
    }
    fn finish(&mut self, b: &Browser, cx: &mut Context<Browser>) {
        let args = std::env::args().collect::<Vec<_>>();
        let path = args
            .iter()
            .position(|a| a == "--report")
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("target/resize-trace.json"));
        let runtime = gpui_wpe::runtime_info();
        let reference = json!({"runtime_version":runtime.version,"library_path":runtime.library_path,"platform":"native Wayland","measurement":"warm local original fixture; diagnostic snapshot readback after submit","presentation_feedback":false,"refresh_rate":"not queried; 60Hz budget reference only","readback_overhead":"diagnostic only; can perturb subsequent samples","cold_load":"excluded from warm resize latency"});
        let elapsed = self
            .continuous
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or(0);
        let report = json!({"schema_version":1,"endpoint":"submitted","reference":reference,"samples":self.samples,"continuous":{"duration_ms":elapsed},"limitations":["Actual compositor presentation time unavailable","CSS marker validates snapshot layout; diagnostic GPU readback can affect pacing","Single compositor/GPU/output scale; broader hardware untested"]});
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_vec_pretty(&report)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| std::fs::write(&path, bytes).map_err(anyhow::Error::from))
        {
            Ok(()) => eprintln!(
                "RESIZE_BENCHMARK trace={} samples={} continuous_ms={}",
                path.display(),
                self.samples.len(),
                elapsed
            ),
            Err(error) => {
                eprintln!("RESIZE_BENCHMARK failed: {error:#}");
                self_test::FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if b.tabs.is_empty() {
            self_test::FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        self.finished = true;
        cx.quit();
    }
    pub fn tick(&mut self, b: &mut Browser, w: &mut Window, cx: &mut Context<Browser>) {
        if self.finished {
            return;
        }
        self.started.get_or_insert_with(Instant::now);
        if self
            .started
            .is_some_and(|t| t.elapsed() > Duration::from_secs(100))
        {
            eprintln!("RESIZE_BENCHMARK timeout");
            self_test::FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
            self.finish(b, cx);
            return;
        }
        let Some(tab) = b.tabs.get(b.active) else {
            return;
        };
        let status = tab.view.read(cx).status();
        if status.loading || status.frames == 0 || status.title != "Resize correctness fixture" {
            return;
        }
        if self.continuous.is_none() {
            if self.request.is_some() && self.capture(b, w, cx, false) {
                self.steps += 1;
            }
            if self.steps >= 200 {
                self.continuous = Some(Instant::now());
                self.last_resize = None;
            } else if self.request.is_none() {
                self.issue(b, w, cx, false, false);
                return;
            }
        }
        if let Some(start) = self.continuous {
            if start.elapsed() < Duration::from_secs(30) {
                if self.request.is_some() {
                    self.capture(b, w, cx, false);
                }
                if self
                    .last_resize
                    .is_none_or(|t| t.elapsed() > Duration::from_millis(50))
                {
                    if self.request.is_some() {
                        self.capture(b, w, cx, true);
                    }
                    self.issue(b, w, cx, true, false);
                }
            } else if self.request.as_ref().is_some_and(|r| r.final_sample) {
                if self.capture(b, w, cx, false) {
                    self.finish(b, cx);
                }
            } else {
                if self.request.is_some() {
                    self.capture(b, w, cx, true);
                }
                self.issue(b, w, cx, false, true);
            }
        }
    }
}
