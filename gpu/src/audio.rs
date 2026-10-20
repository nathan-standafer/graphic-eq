//! Owns the EQ engine, the analyzer and the stream routing.

use crate::analyzer::{Analyzer, Dsp, Levels, Onset, Shared};
use crate::bands;
use crate::clock;
use crate::drums::DrumHit;
use crate::engine::Engine;
use crate::routing::{self, VIRTUAL_SINK};
use crate::synth::{Fixture, Synth, RATE};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A synthetic source in place of `parec`. It never starts the EQ engine or changes routing.
enum SynthSource {
    /// Generated in real time on its own thread (performance runs, interactive use).
    Thread { shared: Arc<Shared>, stop: Arc<AtomicBool> },
    /// Advanced by the UI one frame at a time on a simulated clock (deterministic captures).
    Stepped { dsp: Dsp, synth: Synth, levels: Levels, events: Vec<Onset>, drums: Vec<DrumHit> },
}

pub struct Audio {
    engine: Option<Engine>,
    analyzer: Option<Analyzer>,
    rx: Option<Receiver<(String, Result<Engine, String>)>>,
    redirect: Option<routing::StreamRedirect>,
    orig_default: Option<String>,
    /// Set when the chosen output was not the system default, so the volume keys follow it; put back on quit.
    default_changed: bool,
    pub real_sink: Option<String>,
    pub ready: bool,
    pub status: String,
    source_pre: bool,
    gains: [f32; bands::COUNT],
    last_check: Instant,
    synth: Option<(Fixture, SynthSource)>,
}

impl Audio {
    pub fn new(output: Option<String>, source_pre: bool, gains: [f32; bands::COUNT]) -> Audio {
        let mut current = routing::default_sink();
        if current.as_deref().map_or(false, |c| c.starts_with("geq_")) {
            current = None; // left over from a crash
        }
        let sinks = routing::list_sinks();
        let target = output
            .filter(|o| sinks.iter().any(|(n, _)| n == o))
            .or_else(|| current.clone())
            .or_else(|| sinks.first().map(|(n, _)| n.clone()));
        let mut a = Audio {
            engine: None,
            analyzer: None,
            rx: None,
            redirect: None,
            orig_default: current,
            default_changed: false,
            real_sink: target.clone(),
            ready: false,
            status: String::new(),
            source_pre,
            gains,
            last_check: Instant::now(),
            synth: None,
        };
        match target {
            Some(t) => a.begin(t),
            None => a.status = "No audio output found.".into(),
        }
        a
    }

    /// Synthetic audio only: no EQ engine, no routing changes. `stepped` selects the deterministic
    /// frame-driven source over the real-time one.
    pub fn new_synth(fx: Fixture, replay: Option<Vec<[f32; 2]>>, stepped: bool, gains: [f32; bands::COUNT]) -> Audio {
        let make = move || match replay.clone() {
            Some(frames) => Synth::with_replay(frames),
            None => Synth::new(fx),
        };
        let src = if stepped {
            SynthSource::Stepped { dsp: Dsp::new(), synth: make(), levels: Levels::default(), events: Vec::new(), drums: Vec::new() }
        } else {
            let shared = Arc::new(Shared::default());
            let stop = Arc::new(AtomicBool::new(false));
            let (sh, st) = (shared.clone(), stop.clone());
            std::thread::spawn(move || {
                let mut dsp = Dsp::new();
                let mut synth = make();
                let origin = clock::now();
                let t0 = Instant::now();
                let (mut l, mut r) = (Vec::new(), Vec::new());
                while !st.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(4));
                    let want = (t0.elapsed().as_secs_f64() * RATE) as u64;
                    let have = synth.position();
                    if want <= have {
                        continue;
                    }
                    l.clear();
                    r.clear();
                    synth.render((want - have) as usize, &mut l, &mut r);
                    let t_end = origin + synth.position() as f64 / RATE;
                    let (lv, ev) = dsp.push(&l, &r, t_end);
                    sh.publish(lv, ev, dsp.take_drums());
                }
            });
            SynthSource::Thread { shared, stop }
        };
        Audio {
            engine: None,
            analyzer: None,
            rx: None,
            redirect: None,
            orig_default: None,
            default_changed: false,
            real_sink: None,
            ready: false,
            status: format!("Synthetic audio ({}); EQ engine off", fx.name()),
            source_pre: false,
            gains,
            last_check: Instant::now(),
            synth: Some((fx, src)),
        }
    }

    pub fn is_synth(&self) -> bool {
        self.synth.is_some()
    }

    /// Stepped sources only: advance the simulated clock by `dt` seconds. `now` is the sim time after the step.
    pub fn advance(&mut self, now: f64) {
        if let Some((_, SynthSource::Stepped { dsp, synth, levels, events, drums })) = &mut self.synth {
            let target = (now * RATE as f64).round() as u64;
            let n = target.saturating_sub(synth.position()) as usize;
            if n > 0 {
                let (mut l, mut r) = (Vec::with_capacity(n), Vec::with_capacity(n));
                synth.render(n, &mut l, &mut r);
                let (lv, ev) = dsp.push(&l, &r, synth.position() as f64 / RATE as f64);
                if let Some(lv) = lv {
                    *levels = lv;
                }
                events.extend(ev);
                drums.extend(dsp.take_drums());
            }
        }
    }

    fn begin(&mut self, target: String) {
        self.status = "Starting equalizer…".into();
        self.ready = false;
        let (tx, rx) = mpsc::channel();
        self.rx = Some(rx);
        std::thread::spawn(move || {
            let r = Engine::start(&target);
            let _ = tx.send((target, r));
        });
    }

    fn retarget(&mut self) {
        self.analyzer = None;
        if self.synth.is_some() {
            return;
        }
        let dev = if self.ready && self.source_pre {
            Some(format!("{VIRTUAL_SINK}.monitor"))
        } else {
            self.real_sink.as_ref().map(|s| format!("{s}.monitor"))
        };
        if let Some(d) = dev {
            match Analyzer::start(&d) {
                Ok(a) => self.analyzer = Some(a),
                Err(e) => self.status = e,
            }
        }
    }

    /// Call every frame.
    pub fn poll(&mut self) {
        if let Some(rx) = &self.rx {
            if let Ok((target, result)) = rx.try_recv() {
                self.rx = None;
                self.real_sink = Some(target.clone());
                match result {
                    Ok(engine) => {
                        engine.set_gains(self.gains);
                        self.engine = Some(engine);
                        self.ready = true;
                        self.status.clear();
                        // The default output stays the real one (unless another was chosen) so the volume keys keep
                        // controlling it; streams are sent to the EQ sink as they appear, watcher first so none slips by.
                        if routing::default_sink().as_deref() != Some(&target) {
                            self.default_changed = routing::set_default_sink(&target);
                        }
                        self.redirect = routing::StreamRedirect::start(VIRTUAL_SINK);
                        routing::move_streams(None, VIRTUAL_SINK);
                    }
                    Err(e) => self.status = format!("EQ engine unavailable ({e}). Showing the analyzer only."),
                }
                self.retarget(); // re-open after the default sink moved so it stays on our tap
            }
        }
        if self.ready && self.last_check.elapsed() > Duration::from_secs(2) {
            self.last_check = Instant::now();
            if !self.engine.as_mut().map_or(false, |e| e.running()) {
                self.ready = false;
                self.engine = None;
                self.redirect = None; // the EQ sink is gone, so its streams fall back to the default output
                self.status = "The EQ engine stopped unexpectedly. Audio is back on your normal output.".into();
                self.retarget();
            }
        }
    }

    pub fn set_gains(&mut self, gains: [f32; bands::COUNT]) {
        self.gains = gains;
        if let Some(e) = &self.engine {
            e.set_gains(gains);
        }
    }

    pub fn set_source_pre(&mut self, pre: bool) {
        if pre != self.source_pre {
            self.source_pre = pre;
            self.retarget();
        }
    }

    pub fn set_output(&mut self, name: String) {
        if self.synth.is_some() || self.real_sink.as_deref() == Some(&name) {
            return;
        }
        self.redirect = None;
        self.engine = None;
        self.analyzer = None;
        self.ready = false;
        self.begin(name);
    }

    pub fn levels(&self) -> Option<Levels> {
        match &self.synth {
            Some((_, SynthSource::Stepped { levels, .. })) => Some(levels.clone()),
            Some((_, SynthSource::Thread { shared, .. })) => Some(shared.levels.lock().unwrap().clone()),
            None => self.analyzer.as_ref().map(|a| a.shared.levels.lock().unwrap().clone()),
        }
    }

    /// Onsets found since the last call, oldest first.
    pub fn take_events(&mut self) -> Vec<Onset> {
        match &mut self.synth {
            Some((_, SynthSource::Stepped { events, .. })) => std::mem::take(events),
            Some((_, SynthSource::Thread { shared, .. })) => std::mem::take(&mut *shared.events.lock().unwrap()),
            None => self.analyzer.as_ref().map(|a| std::mem::take(&mut *a.shared.events.lock().unwrap())).unwrap_or_default(),
        }
    }

    /// Drum hits found since the last call, oldest first.
    pub fn take_drums(&mut self) -> Vec<DrumHit> {
        match &mut self.synth {
            Some((_, SynthSource::Stepped { drums, .. })) => std::mem::take(drums),
            Some((_, SynthSource::Thread { shared, .. })) => std::mem::take(&mut *shared.drums.lock().unwrap()),
            None => self.analyzer.as_ref().map(|a| std::mem::take(&mut *a.shared.drums.lock().unwrap())).unwrap_or_default(),
        }
    }

    pub fn shutdown(&mut self) {
        if let Some((_, SynthSource::Thread { stop, .. })) = &self.synth {
            stop.store(true, Ordering::Relaxed);
        }
        self.analyzer = None;
        self.redirect = None;
        if self.ready {
            if let Some(t) = self.real_sink.clone().or_else(|| self.orig_default.clone()) {
                routing::move_streams(Some(VIRTUAL_SINK), &t);
            }
            if let Some(d) = self.orig_default.as_deref().filter(|_| self.default_changed) {
                routing::set_default_sink(d);
            }
        }
        self.engine = None;
        self.ready = false;
    }
}
