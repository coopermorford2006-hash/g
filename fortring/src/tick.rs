//! The per-frame gameplay loop (runs in Elden Ring's ChrIns_PostPhysics task) and the camera write
//! (Draw_Pre). Order: input -> Elden Ring UI state -> camera look -> movement -> health -> building
//! -> guns -> loot -> save.
use crate::generated::*;
use crate::state::{Mode, STATE};
use crate::world::{self, v};
use crate::{building, camera, guns, health, input, loot, movement};
use eldenring::cs::{CSFeManHudState, LadderState};
use glam::Vec3;
use std::sync::Mutex;
use std::time::Instant;

pub struct Frame {
    pub cam: Option<(Vec3, Vec3, f32)>,
    /// Where the Fortnite character is drawn this frame: feet position and facing yaw.
    pub body: Option<(Vec3, f32)>,
    /// The camera the outfit is drawn with (the mod's, or the game's during an interaction); never written
    /// back to the game, unlike `cam`.
    pub view: Option<(Vec3, Vec3, f32)>,
    pub driving: bool,
    pub started: Option<Instant>,
    pub last: Option<Instant>,
    pub last_save: f64,
    pub ui_seen: [bool; 0x46],
    pub loaded_slot: Option<String>,
    /// Elden Ring drives until this time (seconds since boot), after an interaction.
    pub er_hold_until: f64,
    /// Havok position minus block position of the player: changes only when Havok re-centres its origin.
    pub havok_origin: Option<Vec3>,
    /// Camera height smoothing on steps and uneven ground: the camera lags the feet by this much (m),
    /// decaying; feet height of the last frame in block coordinates.
    pub cam_lag_y: f32,
    pub last_feet_y: Option<f32>,
}

pub static FRAME: Mutex<Frame> = Mutex::new(Frame { cam: None, body: None, view: None, driving: false, started: None, last: None, last_save: 0.0, ui_seen: [false; 0x46], loaded_slot: None, er_hold_until: 0.0, havok_origin: None, cam_lag_y: 0.0, last_feet_y: None });

/// True when Elden Ring itself should drive: a menu, map, dialogue, riding Torrent, a ladder, death.
fn er_drives() -> bool {
    let Some(p) = world::player() else { return true };
    let m = &p.chr_ins.modules;
    if m.data.hp <= 0 || m.ride.is_mounted || m.ride.is_mounting || m.ladder.state != LadderState::None {
        return true;
    }
    match world::fe_man().map(|f| f.hud_state) {
        Some(CSFeManHudState::ShowAll) | Some(CSFeManHudState::PopupMenu) => true,
        _ => false,
    }
}

/// Logs Elden Ring UI elements becoming visible/hidden, to map ui_states ids to screens in testing.
fn log_ui(fr: &mut Frame) {
    let Some(mm) = world::menu_man() else { return };
    for (i, s) in mm.ui_states.iter().enumerate() {
        let vis = s.visible();
        if vis != fr.ui_seen[i] {
            fr.ui_seen[i] = vis;
            crate::log!("ui: state {i:#x} {}", if vis { "shown" } else { "hidden" });
        }
    }
}

fn character_key() -> Option<String> {
    let p = world::player()?;
    let gd = unsafe { p.player_game_data.as_ref() };
    let name: String = String::from_utf16_lossy(&gd.character_name).trim_end_matches('\0').chars().filter(|c| c.is_alphanumeric()).collect();
    if name.is_empty() { None } else { Some(name) }
}

pub fn gameplay() {
    let now = Instant::now();
    let mut fr = FRAME.lock().unwrap();
    let started = *fr.started.get_or_insert(now);
    let dt = fr.last.map(|l| (now - l).as_secs_f32()).unwrap_or(1.0 / 60.0).clamp(0.0, 0.1);
    fr.last = Some(now);
    let t = (now - started).as_secs_f64();
    let inp = input::frame();
    log_ui(&mut fr);

    let Some(p) = world::player() else {
        input::MOD_OWNS_INPUT.store(false, std::sync::atomic::Ordering::Relaxed);
        fr.cam = None;
        fr.body = None;
        fr.view = None;
        return;
    };
    let mut st = STATE.lock().unwrap();

    // a new character (or a load) switches the mod's sidecar save
    if let Some(key) = character_key() {
        if fr.loaded_slot.as_deref() != Some(&key) {
            if fr.loaded_slot.is_some() && st.dirty {
                st.save();
            }
            let slot = key.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
            st.load(slot);
            st.yaw = 0.0;
            st.last_hp = -1;
            fr.loaded_slot = Some(key);
        }
    }

    // after an Elden Ring interaction (door, lever, grace, NPC, pickup) the game plays its own animation
    // and moves the character: let it drive for a moment instead of overwriting the position
    // Havok re-centres its origin as the player crosses map tiles (every position jumps ~33 m): move
    // everything the mod keeps in Havok space along with it
    let bp = &p.block_position;
    let origin = v(&p.chr_ins.modules.physics.position) - Vec3::new(bp.x, bp.y, bp.z);
    if let Some(prev) = fr.havok_origin {
        let shift = origin - prev;
        if shift.length() > 1.0 && shift.length() < 2000.0 {
            let mv = |a: &mut [f32; 3]| { a[0] += shift.x; a[1] += shift.y; a[2] += shift.z; };
            st.saved.builds.iter_mut().for_each(|b| mv(&mut b.pos));
            st.pickups.iter_mut().for_each(|q| mv(&mut q.pos));
            st.damage_numbers.iter_mut().for_each(|d| mv(&mut d.0));
            st.tracers.iter_mut().for_each(|t| { mv(&mut t.0); mv(&mut t.1); });
            crate::log!("world: Havok origin moved {shift:.1}; {} builds and {} pickups follow", st.saved.builds.len(), st.pickups.len());
        }
    }
    fr.havok_origin = Some(origin);

    let interacting = t < fr.er_hold_until && !er_drives();
    let er = er_drives() || interacting;
    if let Some(fe) = world::fe_man() {
        if !er && fe.hud_state == CSFeManHudState::Default {
            fe.hud_state = CSFeManHudState::HideAll; // systems:hide_er_hud
        }
    }
    input::MOD_OWNS_INPUT.store(!er, std::sync::atomic::Ordering::Relaxed);
    // every control with an Elden Ring key presses that key (interact was blocked and never forwarded)
    // ...except interact while the mod shows its own prompt (chest, ammo box, pickup): that E is the mod's
    let mod_prompt = st.prompt.is_some();
    for (c, row) in CONTROLS.iter().enumerate() {
        if row.er_key != "none" && row.owner != "mod" {
            let mine = c == CONTROLS_INTERACT && mod_prompt;
            input::passthrough(row, !er && !mine && inp.down(c));
        }
    }
    if !er && !mod_prompt && inp.pressed(CONTROLS_INTERACT) {
        fr.er_hold_until = t + 2.5;
        crate::log!("input: interact passed to Elden Ring; it drives for 2.5 s");
    }
    if er {
        if fr.driving {
            movement::release();
            fr.driving = false;
            p.chr_ins.chr_flags1c5.set_enable_render(true);
        }
        // keep our yaw following the character so control resumes facing the same way
        let q = &p.chr_ins.modules.physics.orientation;
        st.yaw = 2.0 * q.1.atan2(q.3);
        st.pitch = 0.0;
        fr.cam = None;
        fr.body = None;
        fr.view = None;
        if interacting {
            // Elden Ring animates the interaction (door, grace, pickup); keep its character hidden and
            // draw the outfit where it is, seen through the game's own camera
            p.chr_ins.chr_flags1c5.set_enable_render(false);
            fr.body = Some((v(&p.chr_ins.modules.physics.position), st.yaw));
            fr.view = world::camera().map(|c| {
                let m = &c.pers_cam_1.matrix;
                let aspect = if c.pers_cam_1.aspect_ratio > 0.1 { c.pers_cam_1.aspect_ratio } else { 16.0 / 9.0 };
                let hfov = 2.0 * ((c.pers_cam_1.fov * 0.5).tan() * aspect).atan();
                (Vec3::new(m.3.0, m.3.1, m.3.2), Vec3::new(m.2.0, m.2.1, m.2.2), hfov.to_degrees())
            });
        }
        health::tick(&mut st, &inp, dt, 0.0);
        return;
    }
    fr.driving = true;
    // the Fortnite outfit is drawn in its place (hud::draw_outfit); the game may set this back each frame
    p.chr_ins.chr_flags1c5.set_enable_render(false);

    camera::look(&mut st, inp.dx, inp.dy);
    let fall = movement::step(&mut st, &inp, dt);
    health::tick(&mut st, &inp, dt, fall);
    building::refresh(&st);

    let feet = v(&p.chr_ins.modules.physics.position);
    let mut head = feet + Vec3::Y * MOVEMENT_CAPSULE_HEIGHT_V * if st.crouched { 0.7 } else { 0.95 };
    // the feet snap onto uneven ground and steps; ease the camera over those instead of bouncing with them
    // (block coordinates, so Havok re-centring doesn't count as a step)
    let feet_y = p.block_position.y;
    let (_, grounded, _) = movement::speed_now();
    if let Some(last) = fr.last_feet_y {
        let dy = feet_y - last;
        if grounded && dy.abs() < 0.6 {
            fr.cam_lag_y -= dy;
        } else {
            fr.cam_lag_y = 0.0;
        }
    }
    fr.last_feet_y = Some(feet_y);
    fr.cam_lag_y *= (-14.0 * dt).exp();
    head.y += fr.cam_lag_y;
    let (cam_pos, cam_fwd, fov) = camera::pose(&st, head);
    fr.cam = Some((cam_pos, cam_fwd, fov));
    fr.body = Some((feet, st.yaw));
    fr.view = fr.cam;
    crate::audio::set_listener(cam_pos);

    if st.mode == Mode::Build {
        building::tick(&mut st, &inp, dt, t, cam_pos, cam_fwd);
        st.fire_cooldown = (st.fire_cooldown - dt).max(0.0);
    } else {
        building::tick(&mut st, &inp, dt, t, cam_pos, cam_fwd); // Q / F1-F3 switch into build mode
        let muzzle = guns::muzzle_point(feet, st.yaw);
        guns::tick(&mut st, &inp, dt, muzzle, cam_pos, cam_fwd);
    }
    loot::tick(&mut st, &inp, dt, cam_pos, cam_fwd);

    // fade transient HUD bits
    for d in st.damage_numbers.iter_mut() {
        d.2 -= dt;
    }
    st.damage_numbers.retain(|d| d.2 > 0.0);
    for t in st.tracers.iter_mut() {
        t.2 -= dt;
    }
    st.tracers.retain(|t| t.2 > 0.0);
    if let Some((_, left)) = st.message.as_mut() {
        *left -= dt;
    }
    if st.message.as_ref().map(|m| m.1 <= 0.0).unwrap_or(false) {
        st.message = None;
    }
    if st.dirty && t - fr.last_save > 5.0 {
        st.save();
        fr.last_save = t;
    }
}

pub fn camera_write() {
    camera::log_basis_once();
    let cam = FRAME.lock().unwrap().cam;
    if let Some((pos, fwd, fov)) = cam {
        camera::apply(pos, fwd, fov);
    }
}
