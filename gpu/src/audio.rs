//! Owns the EQ engine, the analyzer and the default-sink routing.

use crate::analyzer::{Analyzer, Levels};
use crate::bands;
use crate::engine::Engine;
use crate::routing::{self, VIRTUAL_SINK};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

pub struct Audio {
    engine: Option<Engine>,
    analyzer: Option<Analyzer>,
    rx: Option<Receiver<(String, Result<Engine, String>)>>,
    orig_default: Option<String>,
    pub real_sink: Option<String>,
    pub ready: bool,
    pub status: String,
    source_pre: bool,
    gains: [f32; bands::COUNT],
    last_check: Instant,
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
            orig_default: current,
            real_sink: target.clone(),
            ready: false,
            status: String::new(),
            source_pre,
            gains,
            last_check: Instant::now(),
        };
        match target {
            Some(t) => a.begin(t),
            None => a.status = "No audio output found.".into(),
        }
        a
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
                        routing::set_default_sink(VIRTUAL_SINK);
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
                if let Some(s) = self.real_sink.clone() {
                    routing::set_default_sink(&s);
                }
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
        if self.real_sink.as_deref() == Some(&name) {
            return;
        }
        self.engine = None;
        self.analyzer = None;
        self.ready = false;
        self.begin(name);
    }

    pub fn levels(&self) -> Option<Levels> {
        self.analyzer.as_ref().map(|a| a.latest())
    }

    pub fn shutdown(&mut self) {
        self.analyzer = None;
        if self.ready {
            if let Some(t) = self.real_sink.clone().or_else(|| self.orig_default.clone()) {
                routing::set_default_sink(&t);
                routing::move_streams(Some(VIRTUAL_SINK), &t);
            }
        }
        self.engine = None;
        self.ready = false;
    }
}
