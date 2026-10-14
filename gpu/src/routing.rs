//! Thin wrappers over pactl (PipeWire-pulse / PulseAudio).

use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};

pub const VIRTUAL_SINK: &str = "geq_sink";

fn run(args: &[&str]) -> Option<String> {
    let out = Command::new("pactl").args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn json(args: &[&str]) -> Vec<Value> {
    run(args)
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
}

pub fn default_sink() -> Option<String> {
    run(&["get-default-sink"]).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn set_default_sink(name: &str) -> bool {
    run(&["set-default-sink", name]).is_some()
}

/// Unity gain and unmuted: the volume belongs to the real output, not to our pass-through sink.
pub fn reset_sink_volume(name: &str) {
    run(&["set-sink-volume", name, "100%"]);
    run(&["set-sink-mute", name, "0"]);
}

/// (name, description) of real output devices; our own sink is hidden.
pub fn list_sinks() -> Vec<(String, String)> {
    json(&["--format=json", "list", "sinks"])
        .iter()
        .filter_map(|s| {
            let name = s["name"].as_str()?;
            let desc = s["description"].as_str().unwrap_or(name);
            (!name.starts_with("geq_")).then(|| (name.to_string(), desc.to_string()))
        })
        .collect()
}

fn sink_index(name: &str) -> Option<i64> {
    json(&["--format=json", "list", "sinks"])
        .iter()
        .find(|s| s["name"].as_str() == Some(name))
        .and_then(|s| s["index"].as_i64())
}

/// Move playing streams (from `src`, or all when None) to `dst`, never touching our own geq_* streams.
pub fn move_streams(src: Option<&str>, dst: &str) {
    let src_idx = match src {
        Some(s) => match sink_index(s) {
            Some(i) => Some(i),
            None => return, // that sink is gone; its streams have already fallen back
        },
        None => None,
    };
    let dst_idx = sink_index(dst);
    for i in json(&["--format=json", "list", "sink-inputs"]) {
        let sink = i["sink"].as_i64();
        if (src_idx.is_some() && sink != src_idx) || (sink.is_some() && sink == dst_idx) {
            continue;
        }
        if i["properties"]["node.name"].as_str().map_or(false, |n| n.starts_with("geq_")) {
            continue;
        }
        if let Some(idx) = i["index"].as_i64() {
            run(&["move-sink-input", &idx.to_string(), dst]);
        }
    }
}

/// Sends every new playback stream to `dst` while it lives. The default output stays the real one, so the volume
/// keys and the system volume keep controlling the speakers; a new stream starts on the default output and is moved
/// a moment after it appears.
pub struct StreamRedirect {
    child: Child,
}

impl StreamRedirect {
    pub fn start(dst: &str) -> Option<StreamRedirect> {
        let mut child = Command::new("pactl")
            .arg("subscribe")
            .env("LC_ALL", "C") // the event lines are matched as text
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let out = child.stdout.take()?;
        let dst = dst.to_string();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if line.contains("'new' on sink-input") {
                    move_streams(None, &dst);
                }
            }
        });
        Some(StreamRedirect { child })
    }
}

impl Drop for StreamRedirect {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
