mod analyzer;
mod app;
mod audio;
mod bands;
mod engine;
mod render;
mod routing;
mod settings;

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
        ..Default::default()
    };
    eframe::run_native("Graphic EQ", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
