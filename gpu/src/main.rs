mod analyzer;
mod app;
mod audio;
mod bands;
mod clock;
mod drums;
#[cfg(test)]
mod corpus;
#[cfg(test)]
mod drum_eval;
#[cfg(test)]
mod drum_train;
#[cfg(test)]
mod real_corpus;
mod engine;
mod desktop;
mod eq_strip;
mod modulation;
mod presets;
mod render;
mod routing;
mod scenes;
mod settings;
mod synth;
mod thumbs;

use eframe::egui_wgpu::{WgpuConfiguration, WgpuSetup, WgpuSetupCreateNew};
use eframe::wgpu;
use std::sync::Arc;

/// The default device, plus timestamp queries when asked for (GEQ_GPUTIME=1) and supported.
fn wgpu_config() -> WgpuConfiguration {
    let mut setup = WgpuSetupCreateNew::without_display_handle();
    if std::env::var_os("GEQ_GPUTIME").is_some() {
        let base = setup.device_descriptor.clone();
        setup.device_descriptor = Arc::new(move |adapter: &wgpu::Adapter| {
            let mut d = base(adapter);
            d.required_features |= adapter.features() & wgpu::Features::TIMESTAMP_QUERY;
            d
        });
    }
    WgpuConfiguration { wgpu_setup: WgpuSetup::CreateNew(setup), ..Default::default() }
}

fn main() {
    if let Err(e) = run() {
        eprintln!("SpectraForgeEQ: {e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = args.first().map(String::as_str).unwrap_or("--show");
    if args.len() > 1 {
        return Err("Use one option at a time; see --help".into());
    }
    if arg == "--help" || arg == "-h" {
        println!("SpectraForgeEQ — system equalizer and visualizer\n\nUsage: spectraforgeeq [OPTION]\n  --show          Open the existing window, or start the app (default)\n  --background    Start quietly at login\n  --hide          Hide the window; keep EQ active\n  --toggle        Toggle EQ bypass\n  --enable        Enable EQ\n  --disable       Bypass EQ\n  --preset=N      Select EQ preset 0–9\n  --status        Print status as JSON\n  --quit          Restore normal audio and quit");
        return Ok(());
    }
    let background = arg == "--background";
    let command = match arg {
        "--show" => "show".to_owned(),
        "--background" => "ping".to_owned(),
        "--hide" => "hide".to_owned(),
        "--toggle" => "toggle".to_owned(),
        "--enable" => "on".to_owned(),
        "--disable" => "off".to_owned(),
        "--status" => "status".to_owned(),
        "--quit" => "quit".to_owned(),
        _ => {
            let c = format!("preset {}", arg.strip_prefix("--preset=").ok_or("Unknown option; see --help")?);
            if desktop::parse(&c).is_none() { return Err("Preset must be between 0 and 9".into()); }
            c
        }
    };
    if arg != "--show" && !background {
        println!("{}", desktop::send(&command).map_err(|e| format!("The app is not reachable: {e}"))?);
        return Ok(());
    }
    // Deterministic screenshot tools never touch the real audio or the installed instance.
    let capture = std::env::var_os("GEQ_SHOT").is_some()
        && (std::env::var_os("GEQ_SYNTH").is_some() || std::env::var_os("GEQ_REPLAY").is_some());
    let instance = if capture { None } else {
        match desktop::Instance::acquire()? {
            Some(i) => Some(i),
            None => { desktop::send(&command)?; return Ok(()); }
        }
    };
    let icon = image::load_from_memory(desktop::ICON)?.into_rgba8();
    let icon = eframe::egui::IconData { width: icon.width(), height: icon.height(), rgba: icon.into_raw() };

    let size = std::env::var("GEQ_SIZE").ok().and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some([w.parse().ok()?, h.parse().ok()?])
    });
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title(desktop::NAME)
            .with_app_id("spectraforgeeq")
            .with_icon(icon)
            .with_visible(!background)
            .with_inner_size(size.unwrap_or([1180.0, 760.0]))
            .with_min_inner_size([760.0, 520.0]),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: wgpu_config(),
        ..Default::default()
    };
    eframe::run_native(desktop::NAME, options, Box::new(move |cc| {
        let desktop = instance.map(|i| desktop::Desktop::new(i, cc.egui_ctx.clone()));
        Ok(Box::new(app::App::new(cc, desktop, background)))
    }))?;
    Ok(())
}
