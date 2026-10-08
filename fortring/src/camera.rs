//! systems:camera. Fortnite's over-the-shoulder camera. The mod owns yaw/pitch (mouse) and writes the
//! game's perspective camera matrix every frame after Elden Ring computed its own (Draw_Pre).
use crate::generated::*;
use crate::state::State;
use crate::world;
use glam::Vec3;

/// Forward vector for a yaw/pitch in Elden Ring's Y-up Havok space.
pub fn forward(yaw: f32, pitch: f32) -> Vec3 {
    Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), yaw.cos() * pitch.cos())
}

pub fn flat_forward(yaw: f32) -> Vec3 {
    Vec3::new(yaw.sin(), 0.0, yaw.cos())
}

pub fn flat_right(yaw: f32) -> Vec3 {
    Vec3::new(yaw.cos(), 0.0, -yaw.sin())
}

/// Mouse look; called from the gameplay tick.
pub fn look(st: &mut State, dx: f32, dy: f32) {
    let sens = CAMERA_MOUSE_SENS_V.to_radians() * (1.0 - 0.45 * st.ads);
    st.yaw += dx * sens;
    st.pitch = (st.pitch - dy * sens).clamp(CAMERA_PITCH_MIN_DEG_V.to_radians(), CAMERA_PITCH_MAX_DEG_V.to_radians());
}

/// The camera transform for this frame: (position, forward, fov degrees).
pub fn pose(st: &State, head: Vec3) -> (Vec3, Vec3, f32) {
    let fwd = forward(st.yaw, st.pitch);
    let right = flat_right(st.yaw);
    let [ox, oz, oy] = CAMERA_SHOULDER_OFFSET_M_V;
    let pivot = head + right * ox + flat_forward(st.yaw) * oz + Vec3::Y * oy;
    let boom = CAMERA_BOOM_LENGTH_M_V + (CAMERA_ADS_BOOM_LENGTH_M_V - CAMERA_BOOM_LENGTH_M_V) * st.ads;
    let mut pos = pivot - fwd * boom;
    // keep the camera out of walls
    if let Some(hit) = world::ray(pivot, pos) {
        pos = hit + (pivot - hit).normalize_or_zero() * CAMERA_COLLISION_RADIUS_M_V;
    }
    let ads_fov = st.held_gun().map(|g| g.row().ads_fov).unwrap_or(CAMERA_FOV_V);
    let fov = CAMERA_FOV_V + (ads_fov - CAMERA_FOV_V) * st.ads;
    (pos, fwd, fov)
}

/// Writes the pose into Elden Ring's active perspective camera.
pub fn apply(pos: Vec3, fwd: Vec3, fov_deg: f32) {
    let Some(cam) = world::camera() else { return };
    let right = Vec3::Y.cross(fwd).normalize_or_zero();
    let up = fwd.cross(right);
    let m = &mut cam.pers_cam_1.matrix;
    m.0 = shared_v4(right, 0.0);
    m.1 = shared_v4(up, 0.0);
    m.2 = shared_v4(fwd, 0.0);
    m.3 = shared_v4(pos, 1.0);
    // Elden Ring stores a vertical FOV in radians; Fortnite's 80 is horizontal at 16:9.
    let aspect = if cam.pers_cam_1.aspect_ratio > 0.1 { cam.pers_cam_1.aspect_ratio } else { 16.0 / 9.0 };
    cam.pers_cam_1.fov = 2.0 * ((fov_deg.to_radians() * 0.5).tan() / aspect).atan();
}

fn shared_v4(v: Vec3, w: f32) -> fromsoftware_shared::F32Vector4 {
    fromsoftware_shared::F32Vector4(v.x, v.y, v.z, w)
}

/// Logs the game's own camera basis once, so the handedness used above can be checked in game.
pub fn log_basis_once() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    if let Some(cam) = world::camera() {
        let m = &cam.pers_cam_1.matrix;
        let r = Vec3::new(m.0.0, m.0.1, m.0.2);
        let u = Vec3::new(m.1.0, m.1.1, m.1.2);
        let f = Vec3::new(m.2.0, m.2.1, m.2.2);
        crate::log!("camera: game basis right={r} up={u} fwd={f} (right == up x fwd: {:.2}) fov={}", r.dot(u.cross(f)), cam.pers_cam_1.fov);
    }
}
