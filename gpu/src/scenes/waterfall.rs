use super::*;

/// Spectrum history as a scrolling wireframe terrain in front of a glass wall, standing on a
/// glass floor that reflects both.
pub struct Waterfall {
    floor: wgpu::RenderPipeline,
    terrain_refl: wgpu::RenderPipeline,
    wall_refl: wgpu::RenderPipeline,
    terrain: wgpu::RenderPipeline,
    terrain_stereo: wgpu::RenderPipeline,
    wall: wgpu::RenderPipeline,
}

const CELLS: u32 = (HIST_W - 1) * 191 * 6;
/// The reflection uses every second bin and row.
const REFL_CELLS: u32 = 96 * 96 * 6;

impl Waterfall {
    pub fn new(ctx: &Ctx) -> Waterfall {
        let sh = ctx.module("waterfall", include_str!("waterfall.wgsl"));
        let overlay = Pipe { depth_write: false, always: false, blend: ADDITIVE, msaa: true };
        let glass = Pipe { depth_write: false, always: false, blend: ALPHA, msaa: true };
        // the floor is glass: it does not write depth, so the mirrored scene below it shows through
        let floor = Pipe { depth_write: false, always: false, blend: None, msaa: true };
        Waterfall {
            floor: ctx.pipeline(&sh, "wfloor", "vs_wfloor", "fs_wfloor", floor),
            terrain_refl: ctx.pipeline(&sh, "ter_refl", "vs_ter_refl", "fs_ter_refl", overlay),
            wall_refl: ctx.pipeline(&sh, "wall_refl", "vs_wall_refl", "fs_wall_refl", overlay),
            terrain: ctx.pipeline(&sh, "ter", "vs_ter", "fs_ter", Pipe::OPAQUE),
            // stereo overlay: both channels' wireframes add together, no occlusion between them
            terrain_stereo: ctx.pipeline(&sh, "ter_st", "vs_ter", "fs_ter", overlay),
            wall: ctx.pipeline(&sh, "wall", "vs_wall", "fs_wall", glass),
        }
    }
}

impl Scene for Waterfall {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, f: &Frame) {
        pass.set_pipeline(&self.floor);
        pass.draw(0..6, 0..1);
        // reflections first: the real terrain is opaque and covers them where it stands
        pass.set_pipeline(&self.terrain_refl);
        pass.draw(0..REFL_CELLS, 0..if f.p.mode == 1 { 2 } else { 1 });
        pass.set_pipeline(&self.wall_refl);
        pass.draw(0..6, 0..1);
        if f.p.mode == 1 {
            pass.set_pipeline(&self.terrain_stereo);
            pass.draw(0..CELLS, 0..2); // left, right
        } else {
            pass.set_pipeline(&self.terrain);
            pass.draw(0..CELLS, 0..1);
        }
        pass.set_pipeline(&self.wall);
        pass.draw(0..6, 0..1);
    }

    fn camera(&self, aspect: f32, p: &SceneParams) -> (Mat4, Vec3) {
        let o = Orbit { eye: Vec3::new(0.0, 3.6, 6.5), target: Vec3::new(0.0, -0.2, -9.5), fov_deg: 52.0 };
        orbit(&o, aspect, &p.cam)
    }
}
