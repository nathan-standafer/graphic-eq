//! The equalizer itself: a PipeWire filter-chain exposed as a virtual sink ("SpectraForgeEQ").
//! Gains are changed live with pw-cli; slider drags coalesce into few calls.

use crate::{bands, routing, routing::VIRTUAL_SINK};
use serde_json::Value;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Sender};
use std::time::{Duration, Instant};

fn config(target: &str) -> String {
    let mut nodes = String::new();
    let mut links = String::new();
    for ch in ["L", "R"] {
        for (i, f) in bands::FREQS.iter().enumerate() {
            nodes += &format!(
                "{{ type=builtin name=eq{ch}{i} label=bq_peaking control={{ \"Freq\"={f} \"Q\"={} \"Gain\"=0.0 }} }} ",
                bands::Q
            );
            if i > 0 {
                links += &format!("{{ output=\"eq{ch}{}:Out\" input=\"eq{ch}{i}:In\" }} ", i - 1);
            }
        }
    }
    let last = bands::COUNT - 1;
    format!(
        r#"context.properties = {{ log.level = 0 }}
context.spa-libs = {{ audio.convert.* = audioconvert/libspa-audioconvert  support.* = support/libspa-support }}
context.modules = [
  {{ name = libpipewire-module-rt args = {{ }} flags = [ ifexists nofail ] }}
  {{ name = libpipewire-module-protocol-native }}
  {{ name = libpipewire-module-client-node }}
  {{ name = libpipewire-module-adapter }}
  {{ name = libpipewire-module-filter-chain args = {{
      node.description = "SpectraForgeEQ"
      media.name = "SpectraForgeEQ"
      filter.graph = {{
        nodes = [ {nodes} ]
        links = [ {links} ]
        inputs = [ "eqL0:In" "eqR0:In" ]
        outputs = [ "eqL{last}:Out" "eqR{last}:Out" ]
      }}
      audio.channels = 2
      audio.position = [ FL FR ]
      capture.props = {{ node.name = "{VIRTUAL_SINK}" media.class = Audio/Sink node.always-process = true node.pause-on-idle = false }}
      playback.props = {{ node.name = "geq_out" node.passive = true node.always-process = true node.pause-on-idle = false target.object = "{target}" }}
  }} }}
]
"#
    )
}

fn find_node_id(name: &str) -> Option<u32> {
    let out = Command::new("pw-dump").output().ok()?;
    let v: Value = serde_json::from_slice(&out.stdout).ok()?;
    v.as_array()?.iter().find_map(|o| {
        (o["type"] == "PipeWire:Interface:Node" && o["info"]["props"]["node.name"] == name)
            .then(|| o["id"].as_u64().map(|i| i as u32))
            .flatten()
    })
}

pub struct Engine {
    child: Child,
    tx: Sender<[f32; bands::COUNT]>,
}

impl Engine {
    /// Blocking: launch the filter-chain and wait for its sink. Call off the UI thread.
    pub fn start(target: &str) -> Result<Engine, String> {
        let dir = std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| "/tmp".into());
        let path = format!("{dir}/graphic-eq-engine.conf");
        std::fs::write(&path, config(target)).map_err(|e| e.to_string())?;
        let mut child = Command::new("pipewire")
            .args(["-c", &path])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("could not launch pipewire: {e}"))?;
        let deadline = Instant::now() + Duration::from_secs(6);
        let node = loop {
            if let Ok(Some(_)) = child.try_wait() {
                return Err("engine exited".into());
            }
            if let Some(id) = find_node_id(VIRTUAL_SINK) {
                break id;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                return Err("timed out waiting for the EQ sink".into());
            }
            std::thread::sleep(Duration::from_millis(150));
        };
        // WirePlumber restores the sink's last-saved volume as it appears (the volume keys change it while it is the
        // default sink, and that gets saved). It stacks on the real output's own volume, so the EQ would come up
        // quieter than the bare output with every band at 0 dB. The real output keeps the level; this sink stays at unity.
        // Twice, in case WirePlumber's restore lands after the first.
        routing::reset_sink_volume(VIRTUAL_SINK);
        std::thread::sleep(Duration::from_millis(250));
        routing::reset_sink_volume(VIRTUAL_SINK);
        let (tx, rx) = mpsc::channel::<[f32; bands::COUNT]>();
        std::thread::spawn(move || {
            while let Ok(mut gains) = rx.recv() {
                while let Ok(g) = rx.try_recv() {
                    gains = g; // latest wins
                }
                let mut params = String::new();
                for (i, g) in gains.iter().enumerate() {
                    for ch in ["L", "R"] {
                        params += &format!("\"eq{ch}{i}:Gain\" {g:.2} ");
                    }
                }
                let _ = Command::new("pw-cli")
                    .args(["s", &node.to_string(), "Props", &format!("{{ params = [ {params}] }}")])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        Ok(Engine { child, tx })
    }

    pub fn set_gains(&self, gains: [f32; bands::COUNT]) {
        let _ = self.tx.send(gains);
    }

    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink_volume_percent(name: &str) -> Option<String> {
        let out = Command::new("pactl").args(["--format=json", "list", "sinks"]).output().ok()?;
        let v: Value = serde_json::from_slice(&out.stdout).ok()?;
        let s = v.as_array()?.iter().find(|s| s["name"] == name)?;
        s["volume"]["front-left"]["value_percent"].as_str().map(str::to_string)
    }

    /// Unloads the null sink and stops the player however the test ends.
    struct Cleanup(String, Option<Child>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            if let Some(c) = &mut self.1 {
                let _ = c.kill();
                let _ = c.wait();
            }
            let _ = Command::new("pactl").args(["unload-module", &self.0]).status();
        }
    }

    /// Needs a running PipeWire. Plays to a silent null sink only. A new stream must end up on the EQ sink while the
    /// system default output stays exactly where it was: `cargo test --release new_streams_follow_the_eq -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn new_streams_follow_the_eq() {
        let before = routing::default_sink().expect("a default sink");
        let m = Command::new("pactl").args(["load-module", "module-null-sink", "sink_name=geq_test_null"]).output().expect("pactl");
        let mut guard = Cleanup(String::from_utf8_lossy(&m.stdout).trim().to_string(), None);
        let _engine = Engine::start("geq_test_null").expect("engine");
        let _redirect = routing::StreamRedirect::start(VIRTUAL_SINK).expect("redirect");
        // 4 s of silence, 48 kHz stereo 16 bit
        let data = 48000 * 4 * 4;
        let mut wav = b"RIFF".to_vec();
        for chunk in [&(36 + data as u32).to_le_bytes()[..], b"WAVEfmt ", &16u32.to_le_bytes(), &1u16.to_le_bytes(), &2u16.to_le_bytes(),
            &48000u32.to_le_bytes(), &(48000u32 * 4).to_le_bytes(), &4u16.to_le_bytes(), &16u16.to_le_bytes(), b"data", &(data as u32).to_le_bytes()] {
            wav.extend_from_slice(chunk);
        }
        wav.resize(wav.len() + data, 0);
        let path = std::env::temp_dir().join("geq-test-silence.wav");
        std::fs::write(&path, wav).unwrap();
        let t0 = Instant::now();
        // Explicit device: WirePlumber would otherwise put a player it saw being moved before straight onto the EQ sink.
        guard.1 = Some(Command::new("paplay").args(["--client-name=geq-test", "--device", &before]).arg(&path).spawn().expect("paplay"));
        let sink_of_player = || -> Option<String> {
            let sinks: Value = serde_json::from_slice(&Command::new("pactl").args(["--format=json", "list", "sinks"]).output().ok()?.stdout).ok()?;
            let inputs: Value = serde_json::from_slice(&Command::new("pactl").args(["--format=json", "list", "sink-inputs"]).output().ok()?.stdout).ok()?;
            let i = inputs.as_array()?.iter().find(|i| i["properties"]["application.name"] == "geq-test")?;
            sinks.as_array()?.iter().find(|s| s["index"] == i["sink"]).and_then(|s| s["name"].as_str().map(str::to_string))
        };
        let mut on = None;
        while t0.elapsed() < Duration::from_secs(3) {
            on = sink_of_player();
            if on.as_deref() == Some(VIRTUAL_SINK) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        println!("stream reached {:?} after {:?}", on, t0.elapsed());
        assert_eq!(on.as_deref(), Some(VIRTUAL_SINK));
        assert_eq!(routing::default_sink().as_deref(), Some(before.as_str()), "the default output must not change");
    }

    /// Needs a running PipeWire. WirePlumber restores the sink's last-saved volume, so leave it at 80% once and
    /// check that the next engine still starts at unity: `cargo test --release sink_starts_at_unity -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn sink_starts_at_unity() {
        let target = routing::default_sink().filter(|s| !s.starts_with("geq_")).expect("a real default sink");
        {
            let _e = Engine::start(&target).expect("engine");
            assert_eq!(sink_volume_percent(VIRTUAL_SINK).as_deref(), Some("100%"));
            let _ = Command::new("pactl").args(["set-sink-volume", VIRTUAL_SINK, "80%"]).status();
            std::thread::sleep(Duration::from_millis(2500)); // WirePlumber saves ~1 s after a change
        }
        std::thread::sleep(Duration::from_millis(500));
        let _e = Engine::start(&target).expect("engine");
        assert_eq!(sink_volume_percent(VIRTUAL_SINK).as_deref(), Some("100%"));
    }
}
