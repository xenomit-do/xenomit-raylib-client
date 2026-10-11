//! World<->screen transform, following the hero — `MapView2D` viewport
//! equivalent.

/// Zoom clamp range for the map camera.
pub const MIN_ZOOM: f32 = 0.4;
pub const MAX_ZOOM: f32 = 2.5;
const ORBIT_DISTANCE: f32 = 1740.0;
const FOVY_DEG: f32 = 30.0;
const TILT_RAD: f32 = 135.0_f32.to_radians();

#[derive(Clone, Debug)]
pub struct Camera {
    /// World coordinate at screen center.
    pub center: (f32, f32),
    pub zoom: f32,
    pub viewport: (f32, f32),
    /// DarkOrbit perspective orbit camera for the 3D scene mode.
    pub perspective: bool,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            center: (0.0, 0.0),
            zoom: 1.0,
            viewport: (1280.0, 720.0),
            perspective: false,
        }
    }
}

impl Camera {
    /// Screen pixels per world unit at map point `p`: the plain zoom in 2D,
    /// the perspective ground scale (`focal / depth`) in the 3D camera mode.
    /// 2D sprites drawn at `to_screen(p)` use this so they stay the same size
    /// as the 3D meshes around them.
    #[inline]
    pub fn scale_at(&self, p: (f32, f32)) -> f32 {
        if !self.perspective {
            return self.zoom;
        }
        let distance = ORBIT_DISTANCE / self.zoom.max(0.05);
        let depth = (distance - (p.1 - self.center.1) * TILT_RAD.sin()).max(distance * 0.1);
        let focal = self.viewport.1 * 0.5 / (FOVY_DEG.to_radians() * 0.5).tan();
        focal / depth
    }

    /// World -> screen px.
    #[inline]
    pub fn to_screen(&self, p: (f32, f32)) -> (f32, f32) {
        if self.perspective {
            return self.to_screen_perspective(p);
        }
        (
            (p.0 - self.center.0) * self.zoom + self.viewport.0 * 0.5,
            (p.1 - self.center.1) * self.zoom + self.viewport.1 * 0.5,
        )
    }

    /// Screen px -> world.
    #[inline]
    pub fn to_world(&self, p: (f32, f32)) -> (f32, f32) {
        if self.perspective {
            return self.to_world_perspective(p);
        }
        (
            (p.0 - self.viewport.0 * 0.5) / self.zoom + self.center.0,
            (p.1 - self.viewport.1 * 0.5) / self.zoom + self.center.1,
        )
    }

    fn to_screen_perspective(&self, p: (f32, f32)) -> (f32, f32) {
        let distance = ORBIT_DISTANCE / self.zoom.max(0.05);
        let a = distance * TILT_RAD.sin();
        let dx = p.0 - self.center.0;
        let dy = p.1 - self.center.1;
        let depth = distance - dy * TILT_RAD.sin();
        if depth <= 1.0 {
            return (f32::NAN, f32::NAN);
        }
        let focal = self.viewport.1 * 0.5 / (FOVY_DEG.to_radians() * 0.5).tan();
        // Away3D is left-handed: +x map is screen-right.
        let view_x = dx;
        let view_y = dy * TILT_RAD.cos();
        let _ = a; // documents the matching observer orbit radius
        (
            self.viewport.0 * 0.5 + focal * view_x / depth,
            self.viewport.1 * 0.5 - focal * view_y / depth,
        )
    }

    fn to_world_perspective(&self, p: (f32, f32)) -> (f32, f32) {
        // Cast the camera ray through the pixel and intersect the map plane y=0.
        let distance = ORBIT_DISTANCE / self.zoom.max(0.05);
        let a = distance * TILT_RAD.sin();
        let camera_y = a;
        let camera_z = -self.center.1 - a;
        let focal = self.viewport.1 * 0.5 / (FOVY_DEG.to_radians() * 0.5).tan();
        let nx = (p.0 - self.viewport.0 * 0.5) / focal;
        let ny = -(p.1 - self.viewport.1 * 0.5) / focal;
        // D + right*nx + up*ny, for pan=0 / tilt=135° (right = +x, Away3D LH).
        let dir_x = nx;
        let dir_y = TILT_RAD.cos() + ny * TILT_RAD.sin();
        let dir_z = TILT_RAD.sin() - ny * TILT_RAD.cos();
        if dir_y.abs() < 1e-6 {
            return (self.center.0, self.center.1);
        }
        let t = -camera_y / dir_y;
        let world_x = self.center.0 + dir_x * t;
        let world_z = camera_z + dir_z * t;
        (world_x, -world_z)
    }

    /// Convert a screen-space joystick direction into a normalized map-space
    /// direction at `origin`, including the perspective camera's handedness.
    pub fn screen_direction_to_world(&self, origin: (f32, f32), delta: (f32, f32)) -> (f32, f32) {
        let len = delta.0.hypot(delta.1);
        if len <= f32::EPSILON {
            return (0.0, 0.0);
        }
        let anchor = self.to_screen(origin);
        let sample = self.to_world((
            anchor.0 + delta.0 / len * 10.0,
            anchor.1 + delta.1 / len * 10.0,
        ));
        let vector = (sample.0 - origin.0, sample.1 - origin.1);
        let world_len = vector.0.hypot(vector.1);
        if world_len <= f32::EPSILON || !world_len.is_finite() {
            return (0.0, 0.0);
        }
        (vector.0 / world_len, vector.1 / world_len)
    }

    /// World-space AABB currently on screen, padded by `margin` px.
    /// Returns `(min_x, min_y, max_x, max_y)` in world units.
    pub fn visible_rect(&self, margin_px: f32) -> (f32, f32, f32, f32) {
        if self.perspective {
            let m = margin_px.max(0.0);
            let corners = [
                (-m, -m),
                (self.viewport.0 + m, -m),
                (-m, self.viewport.1 + m),
                (self.viewport.0 + m, self.viewport.1 + m),
            ]
            .map(|point| self.to_world(point));
            if corners
                .iter()
                .all(|point| point.0.is_finite() && point.1.is_finite())
            {
                return (
                    corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min),
                    corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min),
                    corners
                        .iter()
                        .map(|p| p.0)
                        .fold(f32::NEG_INFINITY, f32::max),
                    corners
                        .iter()
                        .map(|p| p.1)
                        .fold(f32::NEG_INFINITY, f32::max),
                );
            }
            // The viewport intersects the horizon; disable culling safely.
            return (
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
                f32::INFINITY,
                f32::INFINITY,
            );
        }
        let m = margin_px / self.zoom.max(0.01);
        let hw = self.viewport.0 * 0.5 / self.zoom;
        let hh = self.viewport.1 * 0.5 / self.zoom;
        (
            self.center.0 - hw - m,
            self.center.1 - hh - m,
            self.center.0 + hw + m,
            self.center.1 + hh + m,
        )
    }

    /// True if a world point is inside the padded viewport.
    pub fn in_view(&self, p: (f32, f32), margin_px: f32) -> bool {
        if self.perspective {
            let (x, y) = self.to_screen(p);
            return x.is_finite()
                && y.is_finite()
                && x >= -margin_px
                && y >= -margin_px
                && x <= self.viewport.0 + margin_px
                && y <= self.viewport.1 + margin_px;
        }
        let (x0, y0, x1, y1) = self.visible_rect(margin_px);
        p.0 >= x0 && p.0 <= x1 && p.1 >= y0 && p.1 <= y1
    }

    /// True if a world-space AABB `(min, max)` intersects the padded
    /// viewport — used to cull visuals whose center is off-screen but
    /// whose bounds still reach into view.
    pub fn rect_in_view(&self, min: (f32, f32), max: (f32, f32), margin_px: f32) -> bool {
        if self.perspective {
            let corners = [min, (max.0, min.1), (min.0, max.1), max].map(|p| self.to_screen(p));
            if corners.iter().all(|p| p.0.is_finite() && p.1.is_finite()) {
                let x0 = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
                let y0 = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
                let x1 = corners
                    .iter()
                    .map(|p| p.0)
                    .fold(f32::NEG_INFINITY, f32::max);
                let y1 = corners
                    .iter()
                    .map(|p| p.1)
                    .fold(f32::NEG_INFINITY, f32::max);
                return x1 >= -margin_px
                    && y1 >= -margin_px
                    && x0 <= self.viewport.0 + margin_px
                    && y0 <= self.viewport.1 + margin_px;
            }
            return true;
        }
        let (x0, y0, x1, y1) = self.visible_rect(margin_px);
        max.0 >= x0 && min.0 <= x1 && max.1 >= y0 && min.1 <= y1
    }

    /// Track the hero without a second, independent camera easing animation.
    pub fn follow(&mut self, target: (f32, f32), _dt: f32) {
        self.center = target;
    }

    /// Mouse-wheel zoom, clamped to `MIN_ZOOM..=MAX_ZOOM`.
    pub fn zoom_by(&mut self, wheel: f32) {
        if wheel.abs() < f32::EPSILON {
            return;
        }
        let next = self.zoom * (1.0 + 0.15 * wheel);
        self.zoom = next.clamp(MIN_ZOOM, MAX_ZOOM);
    }

    /// Pan by a screen-pixel drag delta (converted to world units).
    pub fn pan_pixels(&mut self, delta_px: (f32, f32)) {
        if self.perspective {
            let center_px = (self.viewport.0 * 0.5, self.viewport.1 * 0.5);
            let before = self.to_world(center_px);
            let after = self.to_world((center_px.0 + delta_px.0, center_px.1 + delta_px.1));
            self.center.0 -= after.0 - before.0;
            self.center.1 -= after.1 - before.1;
            return;
        }
        self.center.0 -= delta_px.0 / self.zoom;
        self.center.1 -= delta_px.1 / self.zoom;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_world_roundtrip() {
        let cam = Camera {
            center: (1234.0, -567.0),
            zoom: 1.75,
            viewport: (1600.0, 900.0),
            perspective: false,
        };
        let w = (2000.0, 700.0);
        let s = cam.to_screen(w);
        let back = cam.to_world(s);
        assert!((back.0 - w.0).abs() < 1e-3);
        assert!((back.1 - w.1).abs() < 1e-3);
    }

    #[test]
    fn center_maps_to_viewport_middle() {
        let cam = Camera::default();
        let s = cam.to_screen(cam.center);
        assert!((s.0 - 640.0).abs() < 1e-3);
        assert!((s.1 - 360.0).abs() < 1e-3);
    }

    #[test]
    fn zoom_clamped() {
        let mut cam = Camera::default();
        for _ in 0..50 {
            cam.zoom_by(10.0);
        }
        assert_eq!(cam.zoom, MAX_ZOOM);
        for _ in 0..50 {
            cam.zoom_by(-10.0);
        }
        assert_eq!(cam.zoom, MIN_ZOOM);
    }

    #[test]
    fn rect_in_view_intersects_padded_viewport() {
        // Default camera: center (0,0), 1280x720, zoom 1 → visible
        // rect is (-640, -360, 640, 360) plus margin.
        let cam = Camera::default();
        // Rect fully inside.
        assert!(cam.rect_in_view((-10.0, -10.0), (10.0, 10.0), 0.0));
        // Rect straddling the right edge (min inside, max outside).
        assert!(cam.rect_in_view((600.0, 0.0), (900.0, 50.0), 0.0));
        // Rect fully outside, past the margin.
        assert!(!cam.rect_in_view((800.0, 0.0), (900.0, 50.0), 0.0));
        // Margin rescues a just-off-screen rect.
        assert!(cam.rect_in_view((700.0, 0.0), (900.0, 50.0), 100.0));
        assert!(!cam.rect_in_view((800.0, 0.0), (900.0, 50.0), 100.0));
        // A rect whose *center* is off-screen but overlapping is kept —
        // the big-base case this exists for.
        assert!(cam.rect_in_view((640.0, -200.0), (2000.0, 200.0), 0.0));
    }

    #[test]
    fn follow_converges() {
        let mut cam = Camera::default();
        for _ in 0..120 {
            cam.follow((1000.0, 500.0), 1.0 / 60.0);
        }
        assert!((cam.center.0 - 1000.0).abs() < 1.0);
        assert!((cam.center.1 - 500.0).abs() < 1.0);
    }

    #[test]
    fn perspective_ground_projection_roundtrips_map_points() {
        let cam = Camera {
            center: (5400.0, 6200.0),
            zoom: 1.15,
            viewport: (1920.0, 1080.0),
            perspective: true,
        };
        for point in [(5400.0, 6200.0), (6100.0, 5700.0), (4900.0, 6800.0)] {
            let screen = cam.to_screen(point);
            let back = cam.to_world(screen);
            assert!(
                (back.0 - point.0).abs() < 0.1,
                "x: {point:?} -> {screen:?} -> {back:?}"
            );
            assert!(
                (back.1 - point.1).abs() < 0.1,
                "y: {point:?} -> {screen:?} -> {back:?}"
            );
        }
    }

    #[test]
    fn perspective_projection_matches_raylib_native_camera() {
        let cam = Camera {
            center: (5400.0, 6200.0),
            zoom: 1.15,
            viewport: (1920.0, 1080.0),
            perspective: true,
        };
        let distance = ORBIT_DISTANCE / cam.zoom;
        // raylib is right-handed; Away3D's left-handed (x, y, z) maps to
        // raylib (x, y, -z), i.e. map (x, y) -> (x, 0, y).
        let look = raylib::prelude::Vector3::new(cam.center.0, 0.0, cam.center.1);
        let pos = raylib::prelude::Vector3::new(
            look.x,
            distance * -TILT_RAD.cos(),
            look.z + distance * TILT_RAD.sin(),
        );
        let camera = raylib::prelude::Camera3D::perspective(
            pos,
            look,
            raylib::prelude::Vector3::new(0.0, 1.0, 0.0),
            FOVY_DEG,
        );
        for point in [(5400.0, 6200.0), (6100.0, 5700.0), (4900.0, 6800.0)] {
            let expected = cam.to_screen(point);
            let world = raylib::prelude::Vector3::new(point.0, 0.0, point.1);
            let got = unsafe {
                raylib::ffi::GetWorldToScreenEx(world.into(), (&camera).into(), 1920, 1080)
            };
            assert!(
                (got.x - expected.0).abs() < 0.1,
                "x mismatch {point:?}: native {}, expected {}",
                got.x,
                expected.0
            );
            assert!(
                (got.y - expected.1).abs() < 0.1,
                "y mismatch {point:?}: native {}, expected {}",
                got.y,
                expected.1
            );
        }
    }

    #[test]
    fn screen_right_joystick_direction_moves_right_in_2d_and_3d() {
        for perspective in [false, true] {
            let cam = Camera {
                center: (5000.0, 7000.0),
                zoom: 1.0,
                viewport: (1280.0, 720.0),
                perspective,
            };
            let hero = (5100.0, 6900.0);
            let direction = cam.screen_direction_to_world(hero, (1.0, 0.0));
            let screen =
                cam.to_screen((hero.0 + direction.0 * 100.0, hero.1 + direction.1 * 100.0));
            let start = cam.to_screen(hero);
            assert!(
                screen.0 > start.0,
                "right stick should move right: {direction:?}, {start:?} -> {screen:?}"
            );
        }
    }

    #[test]
    fn perspective_scale_matches_projected_spacing() {
        let cam = Camera {
            center: (5000.0, 5000.0),
            zoom: 1.0,
            viewport: (1920.0, 1080.0),
            perspective: true,
        };
        // Nearer (south, larger map y) points are drawn bigger.
        assert!(cam.scale_at((5000.0, 5400.0)) > cam.scale_at((5000.0, 5000.0)));
        assert!(cam.scale_at((5000.0, 4600.0)) < cam.scale_at((5000.0, 5000.0)));
        // Scale equals the screen distance of a small horizontal step.
        let p = (5000.0, 5300.0);
        let a = cam.to_screen(p);
        let b = cam.to_screen((p.0 + 10.0, p.1));
        assert!(((b.0 - a.0) / 10.0 - cam.scale_at(p)).abs() < 1e-3);
        // 2D mode is the plain zoom.
        let flat = Camera {
            perspective: false,
            zoom: 1.3,
            ..cam
        };
        assert_eq!(flat.scale_at((1.0, 2.0)), 1.3);
    }
}
