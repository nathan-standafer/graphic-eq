//! The equalizer itself: a PipeWire filter-chain exposed as a virtual sink ("Graphic EQ").
//! Gains are changed live with pw-cli; slider drags coalesce into few calls.

use crate::{bands, routing::VIRTUAL_SINK};
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
      node.description = "Graphic EQ"
      media.name = "Graphic EQ"
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
