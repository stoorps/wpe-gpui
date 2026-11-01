//! App diagnostics. Timings end at DOM observation or native submission, never presentation.
use super::*;
use serde_json::{Value, json};
use std::{cell::Cell, collections::HashSet, fs, path::PathBuf, rc::Rc};

pub static COMPLETED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub const DOCUMENT: &str = r#"<!doctype html><meta charset="utf-8"><title>Runtime performance fixture</title>
<style>body{margin:0;font:18px sans-serif;background:repeating-linear-gradient(#fff 0px,#fff 99px,#cadcea 100px,#cadcea 200px)}header{position:fixed;top:0;background:white;padding:16px;width:100%;z-index:1}input{font:inherit;width:60%;padding:8px}main{padding:160px 30px;height:20000px}p{max-width:600px}</style>
<header><label>Native input fixture <input id="bench-input"></label></header><main><h1>Original long document</h1><p>Input, scrolling, idle and lifecycle measurements use this local document. Diagnostic scripts belong to this trusted fixture, never to email.</p></main><script>globalThis.__benchWheel=0;addEventListener("wheel",()=>__benchWheel++,{passive:true});</script>"#;

#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Warm,
    IdleVisible,
    IdleHidden,
    InputSetup,
    Input,
    ScrollSetup,
    Scroll,
    Soak,
    Mixed,
    Settle,
    Done,
}
struct Request {
    stamp: Rc<Cell<i64>>,
    started: Instant,
    expected: String,
    baseline_scroll: f64,
    expected_wheel: u64,
    queries: usize,
}
struct SoakRequest {
    id: usize,
    started: Instant,
    render: bool,
}
#[derive(Clone, Debug, Default)]
struct ProcessSample {
    ticks: u64,
    rss: u64,
    fds: u64,
    threads: u64,
    voluntary: u64,
    involuntary: u64,
    tree_rss: u64,
    tree_fds: u64,
    processes: u64,
}
impl ProcessSample {
    fn json(&self) -> Value {
        json!({"cpu_ticks":self.ticks,"rss_bytes":self.rss,"fds":self.fds,"threads":self.threads,"voluntary_context_switches":self.voluntary,"involuntary_context_switches":self.involuntary,"tree_rss_bytes":self.tree_rss,"tree_fds":self.tree_fds,"processes":self.processes})
    }
}
struct Config {
    cycles: usize,
    samples: usize,
    idle: Duration,
    mixed: Duration,
    timeout: Duration,
}
pub struct Benchmark {
    config: Config,
    report_path: PathBuf,
    progress_path: PathBuf,
    progress_write_failed: bool,
    phase: Phase,
    started: Instant,
    phase_started: Instant,
    idle_before: Option<ProcessSample>,
    idle_native_before: Option<Value>,
    idle_frames: u64,
    idle_window_active: bool,
    idle_window_visible: bool,
    idle_reports: Vec<Value>,
    input: Vec<Value>,
    scroll: Vec<Value>,
    request: Option<Request>,
    scroll_y: f64,
    wheels: u64,
    soak_request: Option<SoakRequest>,
    completed: usize,
    rendered: usize,
    cancelled: usize,
    weak_closed: Vec<WeakEntity<WebView>>,
    before_soak: Option<ProcessSample>,
    native_before_soak: Option<Value>,
    native_after_soak: Option<Value>,
    after_soak: Option<ProcessSample>,
    checkpoints: Vec<Value>,
    peak_buffers: u64,
    peak_imports: u64,
    peak_live_views: u64,
    peak_live_imports: u64,
    peak_live_sources: u64,
    last_submit: Option<(u64, i64)>,
    first_scroll_dispatch_us: Option<i64>,
    scroll_intervals: Vec<f64>,
    unobserved_submissions: u64,
    failures: Vec<String>,
    first_frame_ms: Option<f64>,
    warm_frames: u64,
    warm_geometry: u64,
    warm_quiet_since: Instant,
    warm_checkpoints: Vec<Value>,
    mixed_steps: usize,
    mixed_elapsed: Duration,
}
fn argument(
    args: &[String],
    key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> std::result::Result<u64, String> {
    let Some(index) = args.iter().rposition(|arg| arg == key) else {
        return Ok(default);
    };
    let value = args
        .get(index + 1)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| format!("{key} requires an integer"))?;
    if !(min..=max).contains(&value) {
        return Err(format!("{key} must be between {min} and {max}"));
    }
    Ok(value)
}
fn report_path(args: &[String]) -> PathBuf {
    args.iter()
        .rposition(|arg| arg == "--report")
        .and_then(|index| args.get(index + 1))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/runtime-trace.json"))
}
impl Default for Benchmark {
    fn default() -> Self {
        let args = std::env::args().collect::<Vec<_>>();
        let mut failures = Vec::new();
        let mut get = |key, default, min, max| match argument(&args, key, default, min, max) {
            Ok(value) => value,
            Err(error) => {
                failures.push(error);
                default
            }
        };
        let config = Config {
            cycles: get("--soak-cycles", 1000, 1, 10000) as usize,
            samples: get("--latency-samples", 200, 0, 10000) as usize,
            idle: Duration::from_secs(get("--idle-seconds", 10, 1, 60)),
            mixed: Duration::from_secs(get("--mixed-seconds", 0, 0, 3600)),
            timeout: Duration::from_secs(get("--runtime-timeout-seconds", 1800, 30, 7200)),
        };
        let report_path = report_path(&args);
        let progress_path = report_path.with_extension("progress.jsonl");
        let prepare = (|| -> std::io::Result<()> {
            if let Some(parent) = report_path.parent() {
                fs::create_dir_all(parent)?;
            }
            if report_path.exists() {
                fs::remove_file(&report_path)?;
            }
            fs::write(&progress_path, [])?;
            Ok(())
        })();
        if let Err(error) = prepare {
            failures.push(format!("cannot prepare diagnostic artifacts: {error}"));
        }
        Self {
            config,
            report_path,
            progress_path,
            progress_write_failed: false,
            phase: Phase::Warm,
            started: Instant::now(),
            phase_started: Instant::now(),
            idle_before: None,
            idle_native_before: None,
            idle_frames: 0,
            idle_window_active: false,
            idle_window_visible: false,
            idle_reports: Vec::new(),
            input: Vec::new(),
            scroll: Vec::new(),
            request: None,
            scroll_y: 1000.,
            wheels: 0,
            soak_request: None,
            completed: 0,
            rendered: 0,
            cancelled: 0,
            weak_closed: Vec::new(),
            before_soak: None,
            native_before_soak: None,
            native_after_soak: None,
            after_soak: None,
            checkpoints: Vec::new(),
            peak_buffers: 0,
            peak_imports: 0,
            peak_live_views: 0,
            peak_live_imports: 0,
            peak_live_sources: 0,
            last_submit: None,
            first_scroll_dispatch_us: None,
            scroll_intervals: Vec::new(),
            unobserved_submissions: 0,
            failures,
            first_frame_ms: None,
            warm_frames: 0,
            warm_geometry: 0,
            warm_quiet_since: Instant::now(),
            warm_checkpoints: Vec::new(),
            mixed_steps: 0,
            mixed_elapsed: Duration::ZERO,
        }
    }
}
fn read_number(text: &str, key: &str) -> u64 {
    text.lines()
        .find_map(|line| {
            line.strip_prefix(key)?
                .split_whitespace()
                .next()?
                .parse()
                .ok()
        })
        .unwrap_or(0)
}
fn cpu_ticks(stat: &str) -> Option<u64> {
    // comm is parenthesized and may itself contain spaces or parentheses.
    let fields = stat
        .rsplit_once(')')?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    fields
        .get(11)?
        .parse::<u64>()
        .ok()?
        .checked_add(fields.get(12)?.parse::<u64>().ok()?)
}
fn process_tree(pid: u32, seen: &mut HashSet<u32>, sample: &mut ProcessSample) {
    if seen.len() >= 256 || !seen.insert(pid) {
        return;
    }
    let root = PathBuf::from(format!("/proc/{pid}"));
    let Ok(status) = fs::read_to_string(root.join("status")) else {
        return;
    };
    sample.processes += 1;
    sample.tree_rss += read_number(&status, "VmRSS:") * 1024;
    sample.tree_fds += fs::read_dir(root.join("fd"))
        .map(|entries| entries.count() as u64)
        .unwrap_or(0);
    if let Ok(tasks) = fs::read_dir(root.join("task")) {
        for task in tasks.flatten() {
            if let Ok(children) = fs::read_to_string(task.path().join("children")) {
                for child in children
                    .split_whitespace()
                    .filter_map(|pid| pid.parse().ok())
                {
                    process_tree(child, seen, sample);
                }
            }
        }
    }
}
fn snapshot() -> std::result::Result<ProcessSample, String> {
    let stat = fs::read_to_string("/proc/self/stat").map_err(|e| e.to_string())?;
    let status = fs::read_to_string("/proc/self/status").map_err(|e| e.to_string())?;
    let mut sample = ProcessSample {
        ticks: cpu_ticks(&stat).ok_or("cannot parse process CPU ticks")?,
        rss: read_number(&status, "VmRSS:") * 1024,
        fds: fs::read_dir("/proc/self/fd")
            .map_err(|e| e.to_string())?
            .count() as u64,
        threads: read_number(&status, "Threads:"),
        ..Default::default()
    };
    for task in fs::read_dir("/proc/self/task")
        .map_err(|e| e.to_string())?
        .flatten()
    {
        if let Ok(status) = fs::read_to_string(task.path().join("status")) {
            sample.voluntary += read_number(&status, "voluntary_ctxt_switches:");
            sample.involuntary += read_number(&status, "nonvoluntary_ctxt_switches:");
        }
    }
    process_tree(std::process::id(), &mut HashSet::new(), &mut sample);
    Ok(sample)
}
fn native_snapshot(cx: &App) -> Value {
    let sample = gpui_wpe::live_metrics(cx);
    json!({"views":sample.views,"displays":sample.displays,"imports":sample.imports,"pending_frames":sample.pending_frames,"sources":sample.sources,"resource_waits":sample.resource_waits,"document_bytes":sample.document_bytes,"resource_bytes":sample.resource_bytes})
}
fn percentile(values: &[f64], fraction: f64) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    Some(values[((fraction * values.len() as f64).ceil() as usize).max(1) - 1])
}
fn timings(samples: &[Value]) -> Value {
    let values = samples
        .iter()
        .filter_map(|sample| sample["latency_ms"].as_f64())
        .collect::<Vec<_>>();
    json!({"samples":samples.len(),"matched":samples.iter().filter(|sample|sample["correct"]==true).count(),"p50_ms":percentile(&values,0.5),"p95_ms":percentile(&values,0.95),"p99_ms":percentile(&values,0.99),"max_ms":values.iter().copied().reduce(f64::max),"endpoint":"dispatch-to-DOM-observed upper bound","includes":"GPUI dispatch, WebKit input handling, diagnostic JS IPC, up to 10ms observation polling; excludes compositor presentation"})
}
impl Benchmark {
    pub fn delay(&self) -> Duration {
        if matches!(self.phase, Phase::IdleVisible | Phase::IdleHidden) {
            self.config
                .idle
                .saturating_sub(self.phase_started.elapsed())
                .max(Duration::from_millis(1))
        } else if self.phase == Phase::Mixed {
            Duration::from_millis(200)
        } else {
            Duration::from_millis(10)
        }
    }
    fn transition(&mut self, phase: Phase) {
        if self.phase == Phase::Mixed {
            self.mixed_elapsed = self.phase_started.elapsed();
        }
        self.phase = phase;
        self.phase_started = Instant::now();
    }
    fn sample(&mut self) -> Option<ProcessSample> {
        match snapshot() {
            Ok(sample) => Some(sample),
            Err(error) => {
                self.failures.push(error);
                None
            }
        }
    }
    fn start_idle(&mut self, b: &Browser, window: &Window, cx: &App, phase: Phase) {
        self.idle_window_active = window.is_window_active();
        self.idle_window_visible = window.is_visible();
        self.idle_before = self.sample();
        self.idle_native_before = Some(native_snapshot(cx));
        self.idle_frames = b.tabs[b.active].view.read(cx).status().frames;
        self.transition(phase);
    }
    fn finish_idle(&mut self, b: &Browser, window: &Window, cx: &App) {
        let Some(before) = self.idle_before.take() else {
            return;
        };
        let Some(after) = self.sample() else {
            return;
        };
        let seconds = self.phase_started.elapsed().as_secs_f64();
        // SAFETY: sysconf takes no pointers and returns the process clock-tick rate.
        let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
        let cpu = if ticks_per_second > 0 {
            Some(
                after.ticks.saturating_sub(before.ticks) as f64 / ticks_per_second as f64 / seconds
                    * 100.,
            )
        } else {
            None
        };
        let switches = (after.voluntary as i128 + after.involuntary as i128
            - before.voluntary as i128
            - before.involuntary as i128) as f64
            / seconds;
        let frames = b.tabs[b.active]
            .view
            .read(cx)
            .status()
            .frames
            .saturating_sub(self.idle_frames);
        self.idle_reports.push(json!({"visibility":if self.phase==Phase::IdleVisible{"visible"}else{"hidden"},"window_active_start":self.idle_window_active,"window_active_end":window.is_window_active(),"window_visible_start":self.idle_window_visible,"window_visible_end":window.is_visible(),"duration_seconds":seconds,"process_cpu_percent_one_core":cpu,"process_context_switches_per_second_proxy":switches,"submitted_frames":frames,"before":before.json(),"after":after.json(),"native_before":self.idle_native_before.take(),"native_after":native_snapshot(cx),"diagnostic_timer":"suspended for this entire interval; one boundary callback","limitations":["CPU excludes WebKit subprocesses; tree RSS and FD snapshots include descendants","Context switches are not hardware wakeups; terminating threads can make the aggregate delta negative","Boundary snapshots add small measurement overhead"]}));
    }
    fn begin_query(
        &mut self,
        b: &Browser,
        w: &mut Window,
        cx: &mut Context<Browser>,
        scroll: bool,
    ) {
        let view = b.tabs[b.active].view.clone();
        if scroll && self.first_scroll_dispatch_us.is_none() {
            self.first_scroll_dispatch_us = Some(gpui_wpe::monotonic_us());
        }
        let stamp = Rc::new(Cell::new(0));
        let dispatch_stamp = stamp.clone();
        let count = if scroll {
            self.scroll.len()
        } else {
            self.input.len()
        };
        let expected = if count % 2 == 0 { "x" } else { "y" }.to_string();
        let key = if count % 2 == 0 { "x" } else { "y" };
        let bounds = view.read(cx).viewport_bounds();
        let position = bounds.origin + point(px(120.), px(240.));
        let baseline = self.scroll_y;
        w.defer(cx, move |w, cx| {
            dispatch_stamp.set(gpui_wpe::monotonic_us());
            if scroll {
                w.dispatch_event(
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position,
                        ..Default::default()
                    }),
                    cx,
                );
                w.dispatch_event(
                    PlatformInput::Scroll(ScrollEvent {
                        position,
                        delta: ScrollDelta::Pixels(point(
                            px(0.),
                            px(if count % 2 == 0 { 96. } else { -96. }),
                        )),
                        touch_phase: TouchPhase::Moved,
                        ..Default::default()
                    }),
                    cx,
                );
                w.dispatch_event(
                    PlatformInput::Scroll(ScrollEvent {
                        position,
                        touch_phase: TouchPhase::Ended,
                        ..Default::default()
                    }),
                    cx,
                );
            } else {
                w.dispatch_keystroke(Keystroke::parse("ctrl-a").expect("constant keystroke"), cx);
                w.dispatch_keystroke(Keystroke::parse(key).expect("constant keystroke"), cx);
            }
            let _ = view.read(cx).evaluate(
                "JSON.stringify({value:document.querySelector('input').value,scroll_y:scrollY,wheels:globalThis.__benchWheel||0})",
            );
        });
        self.request = Some(Request {
            stamp,
            started: Instant::now(),
            expected,
            baseline_scroll: baseline,
            expected_wheel: self.wheels + 1,
            queries: 1,
        });
    }
    fn observe_query(&mut self, b: &Browser, cx: &App, scroll: bool) -> bool {
        let view = b.tabs[b.active].view.read(cx);
        let Some(request) = self.request.as_mut() else {
            return false;
        };
        let result = view
            .take_evaluation()
            .and_then(|value| serde_json::from_str::<Value>(&value).ok());
        let correct = result.as_ref().is_some_and(|value| {
            if scroll {
                value["wheels"]
                    .as_u64()
                    .is_some_and(|count| count >= request.expected_wheel)
                    && value["scroll_y"]
                        .as_f64()
                        .is_some_and(|y| (y - request.baseline_scroll).abs() > 0.5)
            } else {
                value["value"].as_str() == Some(&request.expected)
            }
        });
        let timed_out = request.started.elapsed() > Duration::from_millis(500);
        if correct || timed_out {
            let stamp = request.stamp.get();
            let end = gpui_wpe::monotonic_us();
            let sample = json!({"sequence":if scroll{self.scroll.len()}else{self.input.len()},"dispatch_us":stamp,"dom_observed_us":end,"latency_ms":if stamp>0{Some((end-stamp) as f64/1000.)}else{None},"correct":correct,"query_roundtrips":request.queries,"observation":result});
            if let Some(y) = result.as_ref().and_then(|value| value["scroll_y"].as_f64()) {
                self.scroll_y = y;
            }
            if let Some(count) = result.as_ref().and_then(|value| value["wheels"].as_u64()) {
                self.wheels = count;
            }
            if scroll {
                self.scroll.push(sample);
            } else {
                self.input.push(sample);
            }
            self.request = None;
            true
        } else {
            if result.is_some() {
                let _=view.evaluate("JSON.stringify({value:document.querySelector('input').value,scroll_y:scrollY,wheels:globalThis.__benchWheel||0})");
                request.queries += 1;
            }
            false
        }
    }
    fn checkpoint(&mut self, b: &Browser, window: &Window, cx: &App, kind: &str) {
        let Some(sample) = self.sample() else {
            return;
        };
        let native = gpui_wpe::live_metrics(cx);
        let mixed_seconds = if self.phase == Phase::Mixed {
            self.phase_started.elapsed().as_secs_f64()
        } else {
            self.mixed_elapsed.as_secs_f64()
        };
        let event = json!({"kind":kind,"elapsed_seconds":self.started.elapsed().as_secs_f64(),"cycle":self.completed,"mixed_elapsed_seconds":mixed_seconds,"mixed_steps":self.mixed_steps,"active_tabs":b.tabs.len(),"window_active":window.is_window_active(),"sample":sample.json(),"native":native_snapshot(cx)});
        eprintln!(
            "RUNTIME_BENCHMARK checkpoint kind={kind} cycles={} mixed_seconds={mixed_seconds:.1} tabs={} views={} imports={} sources={} fd={} rss_mib={:.2}",
            self.completed,
            b.tabs.len(),
            native.views,
            native.imports,
            native.sources,
            sample.fds,
            sample.rss as f64 / 1048576.
        );
        if !self.progress_write_failed {
            use std::io::Write as _;
            let result = (|| -> anyhow::Result<()> {
                let mut file = fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.progress_path)?;
                serde_json::to_writer(&mut file, &event)?;
                file.write_all(b"\n")?;
                file.flush()?;
                Ok(())
            })();
            if let Err(error) = result {
                self.progress_write_failed = true;
                self.failures
                    .push(format!("progress artifact write failed: {error:#}"));
            }
        }
        self.checkpoints.push(event);
    }
    fn observe_frames(&mut self, b: &Browser, cx: &App) {
        let live = gpui_wpe::live_metrics(cx);
        self.peak_live_views = self.peak_live_views.max(live.views);
        self.peak_live_imports = self.peak_live_imports.max(live.imports);
        self.peak_live_sources = self.peak_live_sources.max(live.sources);
        if let Ok(metrics) = b.tabs[b.active].view.read(cx).metrics() {
            self.peak_buffers = self.peak_buffers.max(metrics.outstanding_frames);
            self.peak_imports = self.peak_imports.max(metrics.cached_imports);
            if self.phase == Phase::Scroll
                && metrics.submitted_frames > 0
                && self
                    .first_scroll_dispatch_us
                    .is_some_and(|start| metrics.submitted_us >= start)
            {
                if let Some((frames, time)) = self.last_submit
                    && metrics.submitted_frames > frames
                {
                    self.scroll_intervals
                        .push((metrics.submitted_us - time) as f64 / 1000.);
                    self.unobserved_submissions +=
                        metrics.submitted_frames.saturating_sub(frames + 1);
                }
                self.last_submit = Some((metrics.submitted_frames, metrics.submitted_us));
            }
        }
    }
    fn start_soak(&mut self, cx: &App) {
        self.before_soak = self.sample();
        self.native_before_soak = Some(native_snapshot(cx));
        self.transition(Phase::Soak);
    }
    fn soak(&mut self, b: &mut Browser, w: &mut Window, cx: &mut Context<Browser>) {
        if let Some(request) = &self.soak_request {
            let Some(tab) = b.tabs.iter().find(|tab| tab.id == request.id) else {
                self.failures.push("soak tab unexpectedly vanished".into());
                self.soak_request = None;
                return;
            };
            let status = tab.view.read(cx).status();
            if request.render
                && status.frames == 0
                && status.error.is_empty()
                && request.started.elapsed() < Duration::from_secs(3)
            {
                return;
            }
            if request.render && status.frames == 0 {
                self.failures.push(format!(
                    "rendered soak cycle {} produced no frame: {}",
                    self.completed, status.error
                ));
            }
            if request.render {
                self.rendered += 1;
            } else {
                self.cancelled += 1;
            }
            self.weak_closed.push(tab.view.downgrade());
            b.command(Command::CloseTab, w, cx);
            self.soak_request = None;
            self.completed += 1;
            if self.completed.is_multiple_of(25) {
                self.checkpoint(b, w, cx, "cycle");
            }
        } else if self.completed < self.config.cycles {
            b.add_tab("demo://runtime-soak/", w, cx);
            if b.error.is_some() {
                self.failures
                    .push(format!("soak creation failed: {:?}", b.error));
                self.transition(Phase::Settle);
                return;
            }
            self.soak_request = Some(SoakRequest {
                id: b.tabs[b.active].id,
                started: Instant::now(),
                render: self.completed.is_multiple_of(5),
            });
        } else if self.config.mixed.is_zero() {
            self.transition(Phase::Settle);
        } else {
            self.transition(Phase::Mixed);
            self.checkpoint(b, w, cx, "mixed_start");
        }
    }
    fn mixed(&mut self, b: &mut Browser, w: &mut Window, cx: &mut Context<Browser>) {
        if self.phase_started.elapsed() >= self.config.mixed {
            b.set_overlay(Overlay::None, w, cx);
            while b.tabs.len() > 1 {
                b.activate(b.tabs.len() - 1, w, cx);
                self.weak_closed.push(b.tabs[b.active].view.downgrade());
                b.command(Command::CloseTab, w, cx);
            }
            self.checkpoint(b, w, cx, "mixed_end");
            self.transition(Phase::Settle);
            return;
        }
        if self.phase_started.elapsed().as_secs() <= self.mixed_steps as u64 {
            return;
        }
        match self.mixed_steps % 6 {
            0 => b.add_tab("demo://runtime-soak/", w, cx),
            1 => b.activate(0, w, cx),
            2 => b.set_overlay(Overlay::Modal, w, cx),
            3 => b.set_overlay(Overlay::None, w, cx),
            4 => b.activate(b.tabs.len() - 1, w, cx),
            _ => {
                if b.tabs.len() > 1 {
                    self.weak_closed.push(b.tabs[b.active].view.downgrade());
                    b.command(Command::CloseTab, w, cx);
                }
            }
        }
        self.mixed_steps += 1;
        if self.mixed_steps.is_multiple_of(60) {
            self.checkpoint(b, w, cx, "mixed");
        }
    }
    fn finish(&mut self, cx: &mut Context<Browser>) {
        let alive = self
            .weak_closed
            .iter()
            .filter(|entity| entity.upgrade().is_some())
            .count();
        let input = timings(&self.input);
        let scroll = timings(&self.scroll);
        let latency_ok = |samples: &[Value], summary: &Value| {
            samples.is_empty()
                || samples.iter().all(|sample| sample["correct"] == true)
                    && summary["p95_ms"].as_f64().is_some_and(|v| v <= 100.)
                    && summary["p99_ms"].as_f64().is_some_and(|v| v <= 200.)
                    && summary["max_ms"].as_f64().is_some_and(|v| v <= 500.)
        };
        let idle_ok = self.idle_reports.len() == 2
            && self.idle_reports.iter().all(|sample| {
                sample["process_cpu_percent_one_core"]
                    .as_f64()
                    .is_some_and(|v| v <= 5.)
                    && sample["process_context_switches_per_second_proxy"]
                        .as_f64()
                        .is_some_and(|v| (0. ..=50.).contains(&v))
                    && sample["submitted_frames"]
                        .as_u64()
                        .is_some_and(|frames| frames <= 2)
            });
        let growth=self.before_soak.as_ref().zip(self.after_soak.as_ref()).map(|(before,after)|json!({"fd_growth":after.fds as i64-before.fds as i64,"rss_growth_bytes":after.rss as i64-before.rss as i64,"tree_fd_growth":after.tree_fds as i64-before.tree_fds as i64,"tree_rss_growth_bytes":after.tree_rss as i64-before.tree_rss as i64,"process_growth":after.processes as i64-before.processes as i64}));
        let resources_ok = growth.as_ref().is_some_and(|g| {
            g["fd_growth"].as_i64().is_some_and(|v| v <= 16)
                && g["rss_growth_bytes"]
                    .as_i64()
                    .is_some_and(|v| v <= 64 * 1024 * 1024)
                && g["tree_fd_growth"].as_i64().is_some_and(|v| v <= 64)
                && g["tree_rss_growth_bytes"]
                    .as_i64()
                    .is_some_and(|v| v <= 256 * 1024 * 1024)
        });
        let scroll_frame_ok = self.scroll.is_empty()
            || !self.scroll_intervals.is_empty()
                && percentile(&self.scroll_intervals, 0.95).is_some_and(|value| value <= 50.)
                && percentile(&self.scroll_intervals, 0.99).is_some_and(|value| value <= 100.)
                && self.scroll_intervals.iter().all(|value| *value <= 200.);
        let mixed_elapsed = if self.phase == Phase::Mixed {
            self.phase_started.elapsed()
        } else {
            self.mixed_elapsed
        };
        let native_ok = self
            .native_before_soak
            .as_ref()
            .zip(self.native_after_soak.as_ref())
            .is_some_and(|(before, after)| {
                ["views", "document_bytes", "resource_bytes"]
                    .iter()
                    .all(|key| after[*key] == before[*key])
                    && ["sources", "pending_frames", "resource_waits"]
                        .iter()
                        .all(|key| {
                            after[*key]
                                .as_u64()
                                .zip(before[*key].as_u64())
                                .is_some_and(|(after, before)| after <= before)
                        })
                    && after["imports"]
                        .as_u64()
                        .zip(after["views"].as_u64())
                        .is_some_and(|(imports, views)| imports <= 8 * views)
                    && after["displays"]
                        .as_u64()
                        .is_some_and(|displays| displays <= 1)
            });
        let proxy_passed = self.failures.is_empty()
            && native_ok
            && self.completed == self.config.cycles
            && alive == 0
            && resources_ok
            && idle_ok
            && latency_ok(&self.input, &input)
            && latency_ok(&self.scroll, &scroll)
            && scroll_frame_ok
            && self.peak_buffers <= 4
            && self.peak_imports <= 8;
        let path = self.report_path.clone();
        let runtime = gpui_wpe::runtime_info();
        let drm=fs::read_dir("/sys/class/drm").ok().into_iter().flatten().flatten().filter_map(|entry| {
            let path=entry.path();
            let vendor=fs::read_to_string(path.join("device/vendor")).ok()?;
            let device=fs::read_to_string(path.join("device/device")).ok();
            Some(json!({"device":entry.file_name().to_string_lossy(),"vendor":vendor.trim(),"pci_device":device.map(|value|value.trim().to_string())}))
        }).collect::<Vec<_>>();
        let cpu_model = fs::read_to_string("/proc/cpuinfo").ok().and_then(|text| {
            text.lines().find_map(|line| {
                line.strip_prefix("model name")
                    .and_then(|line| line.split_once(':'))
                    .map(|(_, value)| value.trim().to_string())
            })
        });
        let reference = json!({
            "cpu_model":cpu_model,
            "logical_cpus":std::thread::available_parallelism().ok().map(|value|value.get()),
            "runtime_version":runtime.version,"library_path":runtime.library_path,
            "kernel":fs::read_to_string("/proc/sys/kernel/osrelease").ok().map(|value|value.trim().to_string()),
            "drm_devices_available_not_active_renderer_proof":drm,
            "wayland_display":std::env::var("WAYLAND_DISPLAY").ok(),
            "software_override":std::env::var("LIBGL_ALWAYS_SOFTWARE").ok(),
            "renderer_override":std::env::var("MESA_LOADER_DRIVER_OVERRIDE").ok()
        });
        let workload = json!({
            "latency_samples_per_kind_requested":self.config.samples,
            "idle_seconds_per_visibility_requested":self.config.idle.as_secs(),
            "minimum_short_suite_workload_requested":self.config.cycles>=1000&&self.config.samples>=200&&self.config.idle.as_secs()>=10,
            "minimum_short_suite_workload_met":self.completed>=1000&&self.input.len()>=200&&self.scroll.len()>=200&&self.idle_reports.len()==2&&self.idle_reports.iter().all(|sample|sample["duration_seconds"].as_f64().is_some_and(|seconds|seconds>=10.)),
            "mixed_observed_seconds":mixed_elapsed.as_secs_f64(),
            "one_hour_mixed_soak_met":mixed_elapsed>=Duration::from_secs(3600)
        });
        let lifecycle = json!({
            "requested_cycles":self.config.cycles,"completed_cycles":self.completed,
            "rendered_cycles":self.rendered,"in_flight_close_cycles":self.cancelled,
            "closed_entities_still_alive":alive,
            "mixed_requested_seconds":self.config.mixed.as_secs(),"mixed_steps":self.mixed_steps,
            "before":self.before_soak.as_ref().map(ProcessSample::json),
            "after":self.after_soak.as_ref().map(ProcessSample::json),"growth":growth,
            "native_before":self.native_before_soak,"native_after":self.native_after_soak,
            "checkpoints":self.checkpoints,"peak_outstanding_buffers":self.peak_buffers,
            "peak_cached_imports":self.peak_imports,
            "peak_live_views_observed":self.peak_live_views,"peak_live_imports_observed":self.peak_live_imports,"peak_live_sources_observed":self.peak_live_sources
        });
        let submission_intervals = json!({
            "p50_ms":percentile(&self.scroll_intervals,0.5),
            "p95_ms":percentile(&self.scroll_intervals,0.95),
            "p99_ms":percentile(&self.scroll_intervals,0.99),
            "samples_ms":self.scroll_intervals,"unobserved_submissions":self.unobserved_submissions,
            "endpoint":"native submission after first dispatched wheel; polling can miss intermediate frames; no compositor timing"
        });
        let budgets = json!({
            "status":"provisional local regression budgets, not compositor presentation budgets",
            "idle_cpu_percent_one_core":5,"context_switch_proxy_per_second":50,"idle_submitted_frames_per_interval":2,
            "dom_observation_ms":{"p95":100,"p99":200,"max":500},
            "scroll_submission_interval_ms":{"p95":50,"p99":100,"max":200},
            "app_fd_growth":16,"app_rss_growth_bytes":64*1024*1024,
            "descendant_fd_growth":64,"descendant_rss_growth_bytes":256*1024*1024,
            "pending_buffers_per_view":4,"cached_imports_per_view":8
        });
        let report = json!({
            "schema_version":1,"benchmark":"runtime-and-lifecycle",
            "reference":reference,"duration_seconds":self.started.elapsed().as_secs_f64(),
            "progress_artifact":self.progress_path,"workload":workload,"first_frame_observed_ms_upper_bound":self.first_frame_ms,"warmup_checkpoints":self.warm_checkpoints,
            "idle":self.idle_reports,
            "input":{"summary":input,"samples":self.input},
            "scroll":{"summary":scroll,"samples":self.scroll,"submission_intervals":submission_intervals},
            "lifecycle":lifecycle,"budgets":budgets,
            "proxy_gates":{"idle":idle_ok,"input":latency_ok(&self.input,&input),"scroll":latency_ok(&self.scroll,&scroll),"resource_growth":resources_ok,"native_live_objects":native_ok,"scroll_submission_intervals":scroll_frame_ok,"passed":proxy_passed},
            "failures":self.failures,
            "limitations":[
                "DOM-observed latency includes diagnostic script IPC/polling; it is not input-to-presentation latency",
                "Submission intervals may include unrelated frames and miss intermediate frames; physical display frame pacing remains unmeasured",
                "Idle measurements suspend the diagnostic timer; CPU excludes WebKit subprocesses and context switches are only a wakeup proxy",
                "RSS includes allocator/driver caches and shared mappings; process-tree RSS double-counts shared pages",
                "FD/RSS budgets are provisional regression gates on this run, not proof of no leak or broad hardware performance",
                "Only the requested soak duration/cycle count was exercised; a short run does not establish the one-hour gate"
            ]
        });
        let result = (|| -> anyhow::Result<()> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&path, serde_json::to_vec_pretty(&report)?)?;
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("RUNTIME_BENCHMARK report failed: {error:#}");
            self_test::FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        if !proxy_passed {
            self_test::FAILED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
        eprintln!(
            "RUNTIME_BENCHMARK trace={} cycles={}/{} proxy_gates_passed={proxy_passed}",
            path.display(),
            self.completed,
            self.config.cycles
        );
        self.phase = Phase::Done;
        COMPLETED.store(true, std::sync::atomic::Ordering::Relaxed);
        cx.quit();
    }
    pub fn tick(&mut self, b: &mut Browser, w: &mut Window, cx: &mut Context<Browser>) {
        if self.phase == Phase::Done {
            return;
        }
        if self.phase == Phase::Warm && !self.failures.is_empty() {
            self.finish(cx);
            return;
        }
        if self.started.elapsed() > self.config.timeout {
            self.failures.push("runtime benchmark timeout".into());
            self.after_soak = self.sample();
            self.native_after_soak = Some(native_snapshot(cx));
            self.finish(cx);
            return;
        }
        if b.tabs.is_empty() {
            self.failures
                .push(format!("cannot create benchmark view: {:?}", b.error));
            self.finish(cx);
            return;
        }
        if matches!(self.phase, Phase::InputSetup | Phase::ScrollSetup)
            && self.phase_started.elapsed() > Duration::from_secs(5)
        {
            self.failures
                .push("diagnostic fixture setup timed out".into());
            self.finish(cx);
            return;
        }
        if self.phase == Phase::Warm && self.phase_started.elapsed() > Duration::from_secs(15) {
            self.failures
                .push("fixture did not reach a quiet idle baseline within15seconds".into());
            self.finish(cx);
            return;
        }
        self.observe_frames(b, cx);
        match self.phase {
            Phase::Warm => {
                let status = b.tabs[b.active].view.read(cx).status();
                if self.warm_checkpoints.len() < (self.phase_started.elapsed().as_secs() as usize) {
                    self.warm_checkpoints.push(json!({"elapsed_ms":self.phase_started.elapsed().as_millis(),"submitted_frames":status.frames,"loading":status.loading,"window_active":w.is_window_active(),"window_visible":w.is_visible(),"native":native_snapshot(cx)}));
                }
                if !status.error.is_empty() {
                    self.failures
                        .push(format!("initial fixture failed: {}", status.error));
                    self.finish(cx);
                    return;
                }
                if self.first_frame_ms.is_none() {
                    w.focus(&b.focus, cx);
                }
                let geometry = b.tabs[b.active]
                    .view
                    .read(cx)
                    .metrics()
                    .map(|metrics| metrics.geometry_generation)
                    .unwrap_or(0);
                if status.frames != self.warm_frames || geometry != self.warm_geometry {
                    self.warm_frames = status.frames;
                    self.warm_geometry = geometry;
                    self.warm_quiet_since = Instant::now();
                }
                if status.frames > 0 && self.first_frame_ms.is_none() {
                    self.first_frame_ms = Some(b.started.elapsed().as_secs_f64() * 1000.);
                    let _ = b.tabs[b.active].view.read(cx).evaluate(
                        "if(document.activeElement)document.activeElement.blur();'blurred'",
                    );
                }
                if status.frames > 0
                    && !status.loading
                    && self.phase_started.elapsed() > Duration::from_secs(2)
                    && self.warm_quiet_since.elapsed() > Duration::from_secs(3)
                {
                    let _ = b.tabs[b.active].view.read(cx).take_evaluation();
                    self.start_idle(b, w, cx, Phase::IdleVisible);
                }
            }
            Phase::IdleVisible => {
                self.finish_idle(b, w, cx);
                let _ = b.tabs[b.active].view.read(cx).set_visible(false);
                self.start_idle(b, w, cx, Phase::IdleHidden);
            }
            Phase::IdleHidden => {
                self.finish_idle(b, w, cx);
                let view = &b.tabs[b.active].view;
                let _ = view.read(cx).set_visible(true);
                w.focus(&view.focus_handle(cx), cx);
                let _ = view
                    .read(cx)
                    .evaluate("document.querySelector('input').focus();'ready'");
                self.transition(Phase::InputSetup);
            }
            Phase::InputSetup => {
                if b.tabs[b.active].view.read(cx).take_evaluation().is_some() {
                    self.transition(Phase::Input);
                }
            }
            Phase::Input => {
                if self.request.is_some() {
                    self.observe_query(b, cx, false);
                } else if self.input.len() < self.config.samples {
                    self.begin_query(b, w, cx, false);
                } else {
                    let _ = b.tabs[b.active].view.read(cx).evaluate(
                        "document.querySelector('input').blur();scrollTo(0,1000);'ready'",
                    );
                    self.transition(Phase::ScrollSetup);
                }
            }
            Phase::ScrollSetup => {
                if self.phase_started.elapsed() > Duration::from_millis(100)
                    && b.tabs[b.active].view.read(cx).take_evaluation().is_some()
                {
                    self.scroll_y = 1000.;
                    self.transition(Phase::Scroll);
                }
            }
            Phase::Scroll => {
                if self.request.is_some() {
                    self.observe_query(b, cx, true);
                } else if self.scroll.len() < self.config.samples {
                    self.begin_query(b, w, cx, true);
                } else {
                    self.start_soak(cx);
                }
            }
            Phase::Soak => self.soak(b, w, cx),
            Phase::Mixed => self.mixed(b, w, cx),
            Phase::Settle => {
                if self.phase_started.elapsed() > Duration::from_secs(3) {
                    let current = native_snapshot(cx);
                    let settled = self.native_before_soak.as_ref().is_some_and(|before| {
                        ["views", "document_bytes", "resource_bytes"]
                            .iter()
                            .all(|key| current[*key] == before[*key])
                            && ["sources", "pending_frames", "resource_waits"]
                                .iter()
                                .all(|key| {
                                    current[*key]
                                        .as_u64()
                                        .zip(before[*key].as_u64())
                                        .is_some_and(|(current, before)| current <= before)
                                })
                    });
                    if settled || self.phase_started.elapsed() > Duration::from_secs(15) {
                        if !settled {
                            self.failures.push("native lifecycle counters did not return to baseline within15seconds".into());
                        }
                        self.after_soak = self.sample();
                        self.native_after_soak = Some(current);
                        self.finish(cx);
                    }
                }
            }
            Phase::Done => {}
        }
    }
}
#[cfg(test)]
mod tests {
    use super::{argument, cpu_ticks, percentile};
    #[test]
    fn parses_proc_comm_with_spaces_and_parentheses() {
        let mut fields = vec!["0"; 22];
        fields[0] = "S";
        fields[11] = "125";
        fields[12] = "75";
        assert_eq!(
            cpu_ticks(&format!("123 (a (b) process) {}", fields.join(" "))),
            Some(200)
        );
        assert_eq!(cpu_ticks("broken"), None);
    }
    #[test]
    fn latency_percentiles_are_nearest_rank() {
        assert_eq!(percentile(&[30., 10., 20.], 0.5), Some(20.));
        assert_eq!(percentile(&[], 0.95), None);
        assert_eq!(percentile(&[30., 10., 20.], 0.99), Some(30.));
    }
    #[test]
    fn bounds_runtime_configuration() {
        let args = vec!["--soak-cycles".into(), "1000".into()];
        assert_eq!(argument(&args, "--soak-cycles", 1, 1, 10000), Ok(1000));
        assert!(argument(&args, "--soak-cycles", 1, 1, 500).is_err());
        let override_args = vec![
            "--soak-cycles".into(),
            "1000".into(),
            "--soak-cycles".into(),
            "20".into(),
        ];
        assert_eq!(
            argument(&override_args, "--soak-cycles", 1, 1, 10000),
            Ok(20)
        );
    }
}
