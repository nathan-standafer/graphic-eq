//! Stored look thumbnails for the look browser. They are real captures (see
//! `tools/make-thumbs.sh`), embedded in the binary, so browsing never runs a scene.

use eframe::egui;

/// In `presets::LOOKS` order.
const DATA: [&[u8]; crate::presets::LOOKS.len()] = [
    include_bytes!("../assets/thumbs/0-studio.png"),
    include_bytes!("../assets/thumbs/0-amber-rack.png"),
    include_bytes!("../assets/thumbs/0-ice-deck.png"),
    include_bytes!("../assets/thumbs/1-neon-terrain.png"),
    include_bytes!("../assets/thumbs/1-classic-mesh.png"),
    include_bytes!("../assets/thumbs/1-ice-sheet.png"),
    include_bytes!("../assets/thumbs/4-ice-geyser.png"),
    include_bytes!("../assets/thumbs/4-ember-fountain.png"),
    include_bytes!("../assets/thumbs/4-neon-spray.png"),
    include_bytes!("../assets/thumbs/4-spectrum-jets.png"),
    include_bytes!("../assets/thumbs/7-obsidian-ice.png"),
    include_bytes!("../assets/thumbs/7-ember-core.png"),
    include_bytes!("../assets/thumbs/7-neon-magnet.png"),
    include_bytes!("../assets/thumbs/7-prism-oil.png"),
    include_bytes!("../assets/thumbs/8-boreal-green.png"),
    include_bytes!("../assets/thumbs/8-polar-ice.png"),
    include_bytes!("../assets/thumbs/8-neon-veil.png"),
    include_bytes!("../assets/thumbs/8-prism-curtains.png"),
    include_bytes!("../assets/thumbs/9-gold-willow.png"),
    include_bytes!("../assets/thumbs/9-ice-chrysanthemum.png"),
    include_bytes!("../assets/thumbs/9-neon-peony.png"),
    include_bytes!("../assets/thumbs/9-prism-finale.png"),
    include_bytes!("../assets/thumbs/10-ice-storm.png"),
    include_bytes!("../assets/thumbs/10-ember-strike.png"),
    include_bytes!("../assets/thumbs/10-neon-arc.png"),
    include_bytes!("../assets/thumbs/10-prism-bolt.png"),
    include_bytes!("../assets/thumbs/11-moonlit-pond.png"),
    include_bytes!("../assets/thumbs/11-lantern-pond.png"),
    include_bytes!("../assets/thumbs/11-neon-canal.png"),
    include_bytes!("../assets/thumbs/11-prism-rain.png"),
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
