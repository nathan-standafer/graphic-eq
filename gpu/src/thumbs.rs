//! Stored look thumbnails for the look browser. They are real captures (see
//! `tools/make-thumbs.sh`), embedded in the binary, so browsing never runs a scene.

use eframe::egui;

/// In `presets::LOOKS` order.
const DATA: [&[u8]; 18] = [
    include_bytes!("../assets/thumbs/0-studio.png"),
    include_bytes!("../assets/thumbs/0-amber-rack.png"),
    include_bytes!("../assets/thumbs/0-ice-deck.png"),
    include_bytes!("../assets/thumbs/1-neon-terrain.png"),
    include_bytes!("../assets/thumbs/1-classic-mesh.png"),
    include_bytes!("../assets/thumbs/1-ice-sheet.png"),
    include_bytes!("../assets/thumbs/2-neon-conduit.png"),
    include_bytes!("../assets/thumbs/2-ice-core.png"),
    include_bytes!("../assets/thumbs/2-ember-flue.png"),
    include_bytes!("../assets/thumbs/2-prism.png"),
    include_bytes!("../assets/thumbs/3-p31-green.png"),
    include_bytes!("../assets/thumbs/3-amber-scope.png"),
    include_bytes!("../assets/thumbs/3-ice-trace.png"),
    include_bytes!("../assets/thumbs/3-neon-lissajous.png"),
    include_bytes!("../assets/thumbs/4-ice-geyser.png"),
    include_bytes!("../assets/thumbs/4-ember-fountain.png"),
    include_bytes!("../assets/thumbs/4-neon-spray.png"),
    include_bytes!("../assets/thumbs/4-spectrum-jets.png"),
];

#[derive(Default)]
pub struct Thumbs {
    tex: Vec<Option<egui::TextureHandle>>,
}

impl Thumbs {
    /// The texture for the look at `index` in the catalog, decoded on first use.
    pub fn get(&mut self, ctx: &egui::Context, index: usize) -> Option<&egui::TextureHandle> {
        if self.tex.len() < DATA.len() {
            self.tex.resize_with(DATA.len(), || None);
        }
        if self.tex[index].is_none() {
            let img = image::load_from_memory(DATA.get(index)?).ok()?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            self.tex[index] = Some(ctx.load_texture(format!("look{index}"), color, egui::TextureOptions::LINEAR));
        }
        self.tex[index].as_ref()
    }
}
