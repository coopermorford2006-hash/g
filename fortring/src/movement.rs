//! systems:movement. A kinematic controller with Fortnite's speeds: it moves the Tarnished by writing
//! the physics position after physics (ChrIns_PostPhysics), using Havok ray casts against the map.
//! While it drives, the game's own gravity is off (CS2 conversion note: otherwise the body builds up a
//! hidden fall speed and the game plays a scripted fall death on landing); we apply our own fall damage.
use crate::camera::{flat_forward, flat_right};
use crate::generated::*;
use crate::input::Frame;
use crate::state::State;
use crate::world::{self, v};
use glam::Vec3;
use std::sync::Mutex;

pub struct Body {
    pub vel: Vec3,
    pub grounded: bool,
    pub fall_start_y: f32,
    pub driving: bool,
    pub saved_gravity: f32,
    pub sprinting: bool,
    /// Where the controller put the character last frame, in Havok space and in block coordinates
    /// (detects teleports by the game; Havok re-centring moves the first but not the second).
    pub last_pos: Option<Vec3>,
    pub last_block: Option<Vec3>,
}

pub static BODY: Mutex<Body> = Mutex::new(Body {
    vel: Vec3::ZERO,
    grounded: true,
    fall_start_y: 0.0,
    driving: false,
    saved_gravity: 1.0,
    sprinting: false,
    last_pos: None,
    last_block: None,
});

/// Hands control back to Elden Ring (menus, riding, ladders, death, cutscenes).
pub fn release() {
    let mut b = BODY.lock().unwrap();
    if !b.driving {
        return;
    }
    b.driving = false;
    b.last_pos = None;
    b.last_block = None;
    if let Some(p) = world::player() {
        let phys = &mut p.chr_ins.modules.physics;
        phys.gravity_multiplier = b.saved_gravity;
        phys.gravity_disabled = false;
        p.chr_ins.modules.fall.disable_fall_motion = false;
    }
    crate::log!("movement: released to Elden Ring");
}

/// One controller step. Returns the fall damage (Fortnite HP) taken on landing, if any.
pub fn step(st: &mut State, inp: &Frame, dt: f32) -> f32 {
    let Some(p) = world::player() else { return 0.0 };
    let mut b = BODY.lock().unwrap();
    // with its gravity off, Elden Ring believes the character is airborne: it plays the falling pose
    // (the "floating" look) and its fall timer runs until it plays a fall death. Keep it grounded; the
    // controller applies Fortnite fall damage itself.
    let block = Vec3::new(p.block_position.x, p.block_position.y, p.block_position.z);
    let air_time = p.chr_ins.modules.fall.fall_timer;
    p.chr_ins.modules.fall.fall_timer = 0.0;
    p.chr_ins.modules.fall.disable_fall_motion = true;
    p.chr_ins.modules.material.disable_fall_damage = true;
    let phys = &mut p.chr_ins.modules.physics;
    if !b.driving {
        b.driving = true;
        b.saved_gravity = phys.gravity_multiplier;
        b.vel = Vec3::ZERO;
        b.fall_start_y = phys.position.1;
        crate::log!("movement: Fortnite controller driving");
    }
    phys.gravity_multiplier = 0.0;
    phys.gravity_disabled = true;

    let mut pos = v(&phys.position);
    // the game moved the character itself (load screen, death warp, grace travel, cutscene): start over
    if let (Some(last), Some(last_block)) = (b.last_pos, b.last_block) {
        if last_block.distance(block) > 3.0 {
            crate::log!("movement: teleported {:.1} m by the game; fall reset", last_block.distance(block));
            b.vel = Vec3::ZERO;
            b.fall_start_y = pos.y;
            b.grounded = false;
        } else if last.distance(pos) > 3.0 {
            // Havok re-centred its origin (every ~32 m of travel): same place, new numbers. Keep the
            // velocity (resetting it stopped the player dead each time) and move the fall height along.
            let shift = (pos - last) - (block - last_block);
            b.fall_start_y += shift.y;
        }
    }
    // input direction relative to the camera
    let mut wish = Vec3::ZERO;
    if inp.down(CONTROLS_MOVE_FWD) { wish += flat_forward(st.yaw); }
    if inp.down(CONTROLS_MOVE_BACK) { wish -= flat_forward(st.yaw) * MOVEMENT_BACKPEDAL_MULT_V; }
    if inp.down(CONTROLS_MOVE_RIGHT) { wish += flat_right(st.yaw); }
    if inp.down(CONTROLS_MOVE_LEFT) { wish -= flat_right(st.yaw); }
    if inp.pressed(CONTROLS_CROUCH) { st.crouched = !st.crouched; }
    b.sprinting = inp.down(CONTROLS_SPRINT) && inp.down(CONTROLS_MOVE_FWD) && st.ads < 0.5 && !st.crouched && st.reload_left <= 0.0;
    let mut speed = if st.crouched { MOVEMENT_CROUCH_SPEED_V } else if b.sprinting { MOVEMENT_SPRINT_SPEED_V } else { MOVEMENT_RUN_SPEED_V };
    speed *= 1.0 - (1.0 - MOVEMENT_ADS_SPEED_MULT_V) * st.ads;
    let target = if wish.length_squared() > 1e-4 { wish.normalize() * speed * wish.length().min(1.0) } else { Vec3::ZERO };

    let accel = if b.grounded { MOVEMENT_ACCEL_V } else { MOVEMENT_ACCEL_V * MOVEMENT_AIR_CONTROL_V };
    let horiz = Vec3::new(b.vel.x, 0.0, b.vel.z);
    let delta = target - horiz;
    let max = accel * dt;
    let horiz = horiz + if delta.length() > max { delta.normalize() * max } else { delta };
    b.vel.x = horiz.x;
    b.vel.z = horiz.z;

    if b.grounded && inp.pressed(CONTROLS_JUMP) {
        b.vel.y = MOVEMENT_JUMP_VELOCITY_V;
        b.grounded = false;
        st.crouched = false;
        b.fall_start_y = pos.y;
    }
    // no collision loaded anywhere below (the area is still streaming in): hold still instead of
    // falling through the unloaded world and taking a lethal "fall" when it appears (died at 46.7 m)
    let world_below = b.grounded || world::map_ray(pos + Vec3::Y * 0.5, pos - Vec3::Y * 200.0).is_some();
    if !world_below {
        b.vel = Vec3::ZERO;
        b.fall_start_y = pos.y;
    } else if !b.grounded {
        b.vel.y = (b.vel.y - MOVEMENT_GRAVITY_V * dt).max(-MOVEMENT_TERMINAL_VELOCITY_V);
    }

    // horizontal move with wall sliding: try the full move, then each axis
    let step_up = Vec3::Y * (MOVEMENT_STEP_HEIGHT_V + 0.05);
    let mv = Vec3::new(b.vel.x, 0.0, b.vel.z) * dt;
    let blocked = |from: Vec3, d: Vec3| -> bool {
        if d.length_squared() < 1e-8 { return false; }
        let dir = d.normalize();
        let reach = d.length() + MOVEMENT_CAPSULE_RADIUS_V;
        [step_up, Vec3::Y * (MOVEMENT_CAPSULE_HEIGHT_V * 0.5), Vec3::Y * (MOVEMENT_CAPSULE_HEIGHT_V - 0.1)]
            .iter()
            .any(|h| world::ray(from + *h, from + *h + dir * reach).is_some())
    };
    for try_mv in [mv, Vec3::new(mv.x, 0.0, 0.0), Vec3::new(0.0, 0.0, mv.z)] {
        if !blocked(pos, try_mv) {
            pos += try_mv;
            break;
        }
    }

    // vertical: ceiling, then ground probe (also handles stepping up small ledges and walking down slopes)
    if b.vel.y > 0.0 {
        let head = pos + Vec3::Y * MOVEMENT_CAPSULE_HEIGHT_V;
        if world::ray(head, head + Vec3::Y * (b.vel.y * dt + 0.05)).is_some() {
            b.vel.y = 0.0;
        }
    }
    pos.y += b.vel.y * dt;
    let probe_top = pos + step_up;
    let snap = if b.grounded { 0.35 } else { 0.0 };
    let probe_bottom = pos - Vec3::Y * (snap + (-b.vel.y * dt).max(0.0) + 0.02);
    let mut fall_damage = 0.0;
    match world::ray(probe_top, probe_bottom) {
        Some(g) if b.vel.y <= 0.0 => {
            if !b.grounded {
                let fell = b.fall_start_y - g.y;
                fall_damage = fall_damage_for(fell);
                if fell > 1.0 {
                    crate::log!("movement: landed after {fell:.1} m (damage {fall_damage:.0})");
                }
            }
            pos.y = g.y;
            b.vel.y = 0.0;
            b.grounded = true;
        }
        _ => {
            if b.grounded {
                b.fall_start_y = pos.y;
            }
            b.grounded = false;
            b.fall_start_y = b.fall_start_y.max(pos.y);
        }
    }

    b.last_pos = Some(pos);
    b.last_block = Some(block + (pos - v(&phys.position)));
    phys.position = world::hp(pos);
    phys.chr_proxy_pos_update_requested = true;
    // the character faces where the camera looks (Fortnite style)
    let half = st.yaw * 0.5;
    let q = eldenring::rotation::Quaternion(0.0, half.sin(), 0.0, half.cos());
    phys.orientation = q;
    phys.interpolated_orientation = q;

    // for in-game testing: where the controller thinks the ground is, every 2 s
    static LAST_TRACE: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    let mut last = LAST_TRACE.lock().unwrap();
    if last.map(|l| l.elapsed().as_secs_f32() > 2.0).unwrap_or(true) {
        *last = Some(std::time::Instant::now());
        let below = world::map_ray(pos + Vec3::Y * 0.5, pos - Vec3::Y * 20.0).map(|g| pos.y - g.y);
        crate::log!(
            "movement: pos {pos:.2} grounded {} vel.y {:.2} speed {:.2} ground below {below:?} (map only), ER fall timer was {air_time:.2}",
            b.grounded, b.vel.y, Vec3::new(b.vel.x, 0.0, b.vel.z).length()
        );
    }
    fall_damage
}

pub fn fall_damage_for(height_m: f32) -> f32 {
    if height_m >= MOVEMENT_FALL_DEATH_V {
        1000.0
    } else if height_m > MOVEMENT_FALL_DAMAGE_START_V {
        (height_m - MOVEMENT_FALL_DAMAGE_START_V) * MOVEMENT_FALL_DAMAGE_PER_M_V
    } else {
        0.0
    }
}

pub fn speed_now() -> (f32, bool, bool) {
    let b = BODY.lock().unwrap();
    (Vec3::new(b.vel.x, 0.0, b.vel.z).length(), b.grounded, b.sprinting)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fall_damage_matches_sheet() {
        assert_eq!(fall_damage_for(5.0), 0.0);
        assert!((fall_damage_for(MOVEMENT_FALL_DAMAGE_START_V + 10.0) - 10.0 * MOVEMENT_FALL_DAMAGE_PER_M_V).abs() < 1e-3);
        assert!(fall_damage_for(MOVEMENT_FALL_DEATH_V) >= 200.0);
    }
    #[test]
    fn jump_apex_is_fortnite_like() {
        // apex = v^2 / 2g; Fortnite's standing jump clears about one metre
        let apex = MOVEMENT_JUMP_VELOCITY_V.powi(2) / (2.0 * MOVEMENT_GRAVITY_V);
        assert!((0.8..1.3).contains(&apex), "apex {apex}");
    }
}
