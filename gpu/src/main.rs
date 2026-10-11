mod analyzer;
mod app;
mod audio;
mod bands;
mod clock;
mod engine;
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

fn main() -> eframe::Result {
    let size = std::env::var("GEQ_SIZE").ok().and_then(|s| {
        let (w, h) = s.split_once('x')?;
        Some([w.parse().ok()?, h.parse().ok()?])
    });
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("Graphic EQ")
            .with_inner_size(size.unwrap_or([1180.0, 760.0]))
            .with_min_inner_size([760.0, 520.0]),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: wgpu_config(),
        ..Default::default()
    };
    eframe::run_native("Graphic EQ", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
