use super::*;

/// Eleven LED columns, one per EQ band: segmented cores inside glass housings on dark plinths,
/// standing on a glossy floor that reflects the lit segments softly.
pub struct Columns {
    floor: wgpu::RenderPipeline,
    reflection: wgpu::RenderPipeline,
    core: wgpu::RenderPipeline,
    plinth: wgpu::RenderPipeline,
    glass: wgpu::RenderPipeline,
}

/// Instance ranges: 11 each of core, glass, plinth, reflection.
const BOX: u32 = 36;

impl Columns {
    pub fn new(ctx: &Ctx) -> Columns {
        let sh = ctx.module("columns", include_str!("columns.wgsl"));
        let floor = Pipe { depth_write: false, always: false, blend: None, msaa: true };
        let reflection = Pipe { depth_write: false, always: false, blend: ADDITIVE, msaa: true };
        let glass = Pipe { depth_write: false, always: false, blend: ALPHA, msaa: true };
        Columns {
            floor: ctx.pipeline(&sh, "col_floor", "vs_floor", "fs_floor", floor),
            reflection: ctx.pipeline(&sh, "col_refl", "vs_col", "fs_refl", reflection),
            core: ctx.pipeline(&sh, "col_core", "vs_col", "fs_core", Pipe::OPAQUE),
            plinth: ctx.pipeline(&sh, "col_plinth", "vs_col", "fs_plinth", Pipe::OPAQUE),
            glass: ctx.pipeline(&sh, "col_glass", "vs_col", "fs_glass", glass),
        }
    }

    /// Column pitch: wide enough to use an ultrawide screen, never wider than the view.
    fn spacing(aspect: f32) -> f32 {
        let visible_half_width = 12.0 * (22f32.to_radians()).tan() * aspect;
        (0.78 * visible_half_width / 5.5).clamp(1.1, 2.6)
    }
}

impl Scene for Columns {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _f: &Frame) {
        pass.set_pipeline(&self.floor);
        pass.draw(0..6, 0..1);
        pass.set_pipeline(&self.reflection);
        pass.draw(0..BOX, 33..44);
        pass.set_pipeline(&self.core);
        pass.draw(0..BOX, 0..11);
        pass.set_pipeline(&self.plinth);
        pass.draw(0..BOX, 22..33);
        pass.set_pipeline(&self.glass);
        pass.draw(0..BOX, 11..22);
    }

    fn camera(&self, aspect: f32, p: &SceneParams) -> (Mat4, Vec3) {
        let o = Orbit { eye: Vec3::new(0.0, 3.6, 12.0), target: Vec3::new(0.0, 3.0, 0.0), fov_deg: 44.0 };
        orbit(&o, aspect, &p.cam)
    }

    fn shape(&self, aspect: f32) -> [f32; 3] {
        [Self::spacing(aspect), 0.0, 0.0]
    }
}
