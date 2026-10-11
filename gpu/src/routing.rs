//! Thin wrappers over pactl (PipeWire-pulse / PulseAudio).

use serde_json::Value;
use std::process::Command;

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
    let src_idx = src.and_then(sink_index);
    for i in json(&["--format=json", "list", "sink-inputs"]) {
        if src_idx.is_some() && i["sink"].as_i64() != src_idx {
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
