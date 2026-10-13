use super::*;

/// Spectrum history as a scrolling wireframe terrain inside a box of glass walls (front, both sides
/// and back), standing on a glass floor under a night sky; the floor mirrors the box and terrain but
/// not the sky.
pub struct Waterfall {
    sky: wgpu::RenderPipeline,
    floor: wgpu::RenderPipeline,
    terrain_refl: wgpu::RenderPipeline,
    wall_refl: wgpu::RenderPipeline,
    side_refl: wgpu::RenderPipeline,
    back_refl: wgpu::RenderPipeline,
    terrain: wgpu::RenderPipeline,
    terrain_stereo: wgpu::RenderPipeline,
    wall: wgpu::RenderPipeline,
    side: wgpu::RenderPipeline,
    back: wgpu::RenderPipeline,
}

const CELLS: u32 = (HIST_W - 1) * 191 * 6;
/// The reflection uses every second bin and row.
const REFL_CELLS: u32 = 96 * 96 * 6;

impl Waterfall {
    pub fn new(ctx: &Ctx) -> Waterfall {
        let sh = ctx.module("waterfall", &format!("{}\n{}", include_str!("sky.wgsl"), include_str!("waterfall.wgsl")));
        let overlay = Pipe { depth_write: false, always: false, blend: ADDITIVE, msaa: true };
        let glass = Pipe { depth_write: false, always: false, blend: ALPHA, msaa: true };
        // the floor is glass: it does not write depth, so the mirrored scene below it shows through
        let floor = Pipe { depth_write: false, always: false, blend: None, msaa: true };
        // the sky is a backdrop painted first, over the part of the screen at or above the horizon
        let sky = Pipe { depth_write: false, always: true, blend: None, msaa: true };
        Waterfall {
            sky: ctx.pipeline(&sh, "sky", "vs_sky", "fs_sky", sky),
            floor: ctx.pipeline(&sh, "wfloor", "vs_wfloor", "fs_wfloor", floor),
            terrain_refl: ctx.pipeline(&sh, "ter_refl", "vs_ter_refl", "fs_ter_refl", overlay),
            wall_refl: ctx.pipeline(&sh, "wall_refl", "vs_wall_refl", "fs_wall_refl", overlay),
            side_refl: ctx.pipeline(&sh, "side_refl", "vs_side_refl", "fs_side_refl", overlay),
            back_refl: ctx.pipeline(&sh, "back_refl", "vs_back_refl", "fs_back_refl", overlay),
            terrain: ctx.pipeline(&sh, "ter", "vs_ter", "fs_ter", Pipe::OPAQUE),
            // stereo overlay: both channels' wireframes add together, no occlusion between them
            terrain_stereo: ctx.pipeline(&sh, "ter_st", "vs_ter", "fs_ter", overlay),
            wall: ctx.pipeline(&sh, "wall", "vs_wall", "fs_wall", glass),
            side: ctx.pipeline(&sh, "side", "vs_side", "fs_side", glass),
            back: ctx.pipeline(&sh, "back", "vs_back", "fs_back", glass),
        }
    }
}

impl Scene for Waterfall {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, f: &Frame) {
        let stereo = f.p.mode == 1;
        pass.set_pipeline(&self.sky);
        pass.draw(0..6, 0..1);
        if !stereo {
            // The opaque terrain goes first so the depth test skips the floor and reflections behind it.
            // That cannot change the picture: along any view ray the real terrain is nearer than the floor
            // and than anything mirrored below it.
            pass.set_pipeline(&self.terrain);
            pass.draw(0..CELLS, 0..1);
        }
        pass.set_pipeline(&self.floor);
        pass.draw(0..6, 0..1);
        // reflections: drawn over the floor, below the glass
        pass.set_pipeline(&self.terrain_refl);
        pass.draw(0..REFL_CELLS, 0..if stereo { 2 } else { 1 });
        pass.set_pipeline(&self.wall_refl);
        pass.draw(0..6, 0..1);
        pass.set_pipeline(&self.side_refl);
        pass.draw(0..6, 0..2); // left, right
        pass.set_pipeline(&self.back_refl);
        pass.draw(0..6, 0..1);
        if stereo {
            // the stereo wireframes add together with no occlusion, so they cannot go first
            pass.set_pipeline(&self.terrain_stereo);
            pass.draw(0..CELLS, 0..2); // left, right
        }
        // glass back to front
        pass.set_pipeline(&self.back);
        pass.draw(0..6, 0..1);
        pass.set_pipeline(&self.side);
        pass.draw(0..6, 0..2);
        pass.set_pipeline(&self.wall);
        pass.draw(0..6, 0..1);
    }

    fn camera(&self, aspect: f32, p: &SceneParams) -> (Mat4, Vec3) {
        let o = Orbit { eye: Vec3::new(0.0, 3.6, 6.5), target: Vec3::new(0.0, -0.2, -9.5), fov_deg: 52.0 };
        orbit(&o, aspect, &p.cam)
    }
}
