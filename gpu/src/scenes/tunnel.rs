use super::*;

/// Fly down a tube whose wall is the last few seconds of spectrum: bass along the floor, treble
/// along the ceiling, the left channel on the left wall and the right channel on the right.
pub struct Tunnel {
    wall: wgpu::RenderPipeline,
    cap: wgpu::RenderPipeline,
}

/// Two walls, each (NX-1) x (NZ-1+PAD) quads (PAD rows extend the tube past the camera).
const WALL_VERTS: u32 = (HIST_W - 1) * (191 + 36) * 6;

impl Tunnel {
    pub fn new(ctx: &Ctx) -> Tunnel {
        let sh = ctx.module("tunnel", include_str!("tunnel.wgsl"));
        let cap = Pipe { depth_write: false, always: false, blend: ADDITIVE, msaa: true };
        Tunnel {
            wall: ctx.pipeline(&sh, "tun_wall", "vs_tun", "fs_tun", Pipe::OPAQUE),
            cap: ctx.pipeline(&sh, "tun_cap", "vs_cap", "fs_cap", cap),
        }
    }

    /// The tube is a circle at 16:9 and stretches into an ellipse toward 32:9, so the left and
    /// right walls use the width of the screen and the floor and ceiling stay in view.
    fn ellipse(aspect: f32) -> f32 {
        (1.0 + (aspect - 1.78) * 0.55).clamp(1.0, 2.3)
    }
}

impl Scene for Tunnel {
    fn draw(&self, pass: &mut wgpu::RenderPass<'_>, _f: &Frame) {
        pass.set_pipeline(&self.wall);
        pass.draw(0..WALL_VERTS, 0..2); // left wall, right wall
        pass.set_pipeline(&self.cap);
        pass.draw(0..6, 0..1);
    }

    fn camera(&self, aspect: f32, p: &SceneParams) -> (Mat4, Vec3) {
        let c = &p.cam;
        let lim = &info(TUNNEL).cam;
        let yaw = (c.yaw.clamp(-lim.yaw, lim.yaw) + c.idle[0]).clamp(-0.7, 0.7);
        let pitch = (c.pitch.clamp(lim.pitch.0, lim.pitch.1) + c.idle[1]).clamp(-0.45, 0.45);
        let dolly = (c.zoom * (1.0 + c.idle[2])).clamp(lim.zoom.0, lim.zoom.1);
        let eye = Vec3::new((c.pan[0] * 0.004).clamp(-0.9, 0.9), (-c.pan[1] * 0.004).clamp(-0.6, 0.6), 2.6 * dolly);
        let dir = Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos());
        // bank about the view direction; the idle path keeps this small and bounded
        let (sr, cr) = c.idle[3].sin_cos();
        let right = dir.cross(Vec3::Y).normalize();
        let up = right.cross(dir).normalize();
        let up = up * cr + right * sr;
        let fov = 72.0 - 12.0 * ((aspect - 1.6) / 1.9).clamp(0.0, 1.0);
        (perspective(fov.to_radians(), aspect, 0.05, 90.0) * look_at(eye, eye + dir, up), eye)
    }

    fn shape(&self, aspect: f32) -> [f32; 3] {
        [Self::ellipse(aspect), 0.0, 0.0]
    }
}
