//! systems:guns. Fortnite gun handling (fire rate, bloom, ADS, recoil, mags, reloads) with damage
//! delivered by real Elden Ring bullets, so enemies flinch, aggro, die and drop runes on their own.
//! Damage = Fortnite damage x region tier, written into the taken-over AtkParam row before each shot.
use crate::camera;
use crate::generated::*;
use crate::input::Frame;
use crate::state::{Held, State};
use crate::world::{self, v};
use eldenring::cs::{AtkParam_Pc, Bullet};
use glam::Vec3;
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicI64, AtomicPtr, Ordering};

/// The rows picked from er_rows candidates at boot (-1 = none found).
pub static GUN_BULLET: AtomicI64 = AtomicI64::new(-1);
pub static GUN_ATK: AtomicI64 = AtomicI64::new(-1);
pub static BLAST_BULLET: AtomicI64 = AtomicI64::new(-1);
pub static BLAST_ATK: AtomicI64 = AtomicI64::new(-1);

/// Mirror of fromsoftware-rs' BulletSpawnData (its fields are private); same layout, size 0x110.
#[repr(C)]
struct SpawnData {
    owner: eldenring::cs::FieldInsHandle,
    behavior_id: i32,
    magic_id: i32,
    unk10: u32,
    bullet_id: i32,
    goods_id: i32,
    dummy_poly_id: i32,
    target: eldenring::cs::FieldInsHandle,
    unk28: u32,
    unk2c: u32,
    unk30: [f32; 4],
    unk40: u32,
    unk44: u32,
    pad48: [u8; 8],
    acceleration_angle: [f32; 4],
    unk60: [f32; 4],
    angle: [f32; 4],
    position: [f32; 4],
    unk90: u64,
    unk98: u64,
    unka0: u64,
    pada8: [u8; 8],
    unkb0: [u8; 0x50],
    unk100: u8,
    pad101: [u8; 15],
}
const _: () = assert!(std::mem::size_of::<SpawnData>() == std::mem::size_of::<eldenring::cs::BulletSpawnData>());

/// Picks the first existing candidate row for each taken-over param and logs its old values.
pub fn pick_rows() {
    let Some(params) = world::params() else { return };
    for (row, slot) in [(ER_ROWS_GUN_BULLET, &GUN_BULLET), (ER_ROWS_ROCKET_BLAST_BULLET, &BLAST_BULLET)] {
        let r = &ER_ROWS[row];
        for &id in r.candidates {
            if let Some(b) = params.get_mut::<Bullet>(id as u32) {
                crate::log!("er_rows: {} = Bullet {id} (was atk {} life {} speed {})", r.id, b.atk_id_bullet(), b.life(), b.init_vellocity());
                slot.store(id as i64, Ordering::Relaxed);
                break;
            }
        }
    }
    for (row, slot) in [(ER_ROWS_GUN_ATK, &GUN_ATK), (ER_ROWS_ROCKET_BLAST_ATK, &BLAST_ATK)] {
        let r = &ER_ROWS[row];
        for &id in r.candidates {
            if let Some(a) = params.get_mut::<AtkParam_Pc>(id as u32) {
                crate::log!("er_rows: {} = AtkParam_Pc {id} (was phys {})", r.id, a.atk_phys());
                slot.store(id as i64, Ordering::Relaxed);
                break;
            }
        }
    }
    let (b, a) = (GUN_BULLET.load(Ordering::Relaxed), GUN_ATK.load(Ordering::Relaxed));
    if b < 0 || a < 0 {
        crate::log!("er_rows: MISSING gun rows (bullet {b}, atk {a}): guns can't deal damage on this game version");
        return;
    }
    let (bb, ba) = (BLAST_BULLET.load(Ordering::Relaxed), BLAST_ATK.load(Ordering::Relaxed));
    if let Some(bul) = params.get_mut::<Bullet>(b as u32) {
        bul.set_atk_id_bullet(a as i32);
        bul.set_gravity_in_range(0.0);
        bul.set_sfx_id_bullet(-1);
        bul.set_sfx_id_hit(-1);
        bul.set_hit_bullet_id(-1);
    }
    if bb >= 0 && ba >= 0 {
        if let Some(bul) = params.get_mut::<Bullet>(bb as u32) {
            bul.set_atk_id_bullet(ba as i32);
            bul.set_life(0.1);
            bul.set_init_vellocity(0.0);
            bul.set_max_vellocity(0.0);
            bul.set_sfx_id_bullet(-1);
        }
    }
}

fn set_shot(damage: f32, speed: f32, range: f32, radius: f32) -> bool {
    let (b, a) = (GUN_BULLET.load(Ordering::Relaxed), GUN_ATK.load(Ordering::Relaxed));
    let Some(params) = world::params() else { return false };
    if b < 0 || a < 0 {
        return false;
    }
    if let Some(atk) = params.get_mut::<AtkParam_Pc>(a as u32) {
        atk.set_atk_phys(damage.round().clamp(1.0, 65535.0) as u16);
        atk.set_atk_phys_correction(100);
    }
    if let Some(bul) = params.get_mut::<Bullet>(b as u32) {
        // CS2 note gotcha 7: a bullet must live about two frames or it never collides
        let speed = speed.max(range / 0.9).min(2000.0).max(range * 30.0);
        bul.set_init_vellocity(speed);
        bul.set_max_vellocity(speed);
        bul.set_life((range / speed).max(2.0 / 30.0));
        bul.set_hit_radius(radius);
    }
    true
}

/// CSBulletManager::SpawnBullet in eldenring.exe 2.7.1.0 / 2.7.1.1 (fromsoftware-rs rva_ww.rs / rva_jp.rs;
/// its rva module is crate-private).
const SPAWN_BULLET_RVA: usize = 0x3a_2cb0;
/// fromsoftware-rs declares 4 arguments and no result, but calls made that way crash 2.7.1.0 inside
/// SpawnBullet (offset 0x38e62d), for the mod's own shots and when the hook forwarded the game's.
/// So the detour forwards 8 argument slots and the result untouched, and logs the extra ones.
type FnSpawn = unsafe extern "C" fn(usize, usize, usize, usize, usize, usize, usize, usize) -> usize;
static SPAWN_ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
/// The spawn data of a bullet the game made itself (player-owned preferred). A zero-filled SpawnData
/// crashed 2.7.1.0 inside SpawnBullet (read of address -1), so gun shots start from this template.
static TEMPLATE: Mutex<Option<([u8; SPAWN_DATA_SIZE], bool)>> = Mutex::new(None);
const SPAWN_SIGNATURE_KNOWN: bool = false;
const SPAWN_DATA_SIZE: usize = std::mem::size_of::<SpawnData>();
thread_local!(static OURS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) });

unsafe extern "C" fn spawn_detour(mgr: usize, out: usize, data_addr: usize, err: usize, a5: usize, a6: usize, a7: usize, a8: usize) -> usize {
    let data = data_addr as *const SpawnData;
    static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    if CALLS.fetch_add(1, Ordering::Relaxed) < 5 {
        crate::log!("guns: game SpawnBullet(mgr {mgr:#x}, out {out:#x}, data {data_addr:#x}, err {err:#x}, {a5:#x}, {a6:#x}, {a7:#x}, {a8:#x})");
    }
    if !data.is_null() && !OURS.with(|o| o.get()) {
        let bytes: [u8; SPAWN_DATA_SIZE] = unsafe { std::ptr::read(data as *const [u8; SPAWN_DATA_SIZE]) };
        let by_player = world::player().map(|p| unsafe { (*data).owner == p.chr_ins.field_ins_handle }).unwrap_or(false);
        let mut t = TEMPLATE.lock().unwrap();
        if t.is_none() || (by_player && !t.as_ref().unwrap().1) {
            let hex: Vec<String> = bytes.chunks(16).enumerate().map(|(i, c)| format!("  {:03x}: {}", i * 16, c.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" "))).collect();
            crate::log!("guns: bullet template from the game (bullet {}, player-owned {by_player}):\n{}", unsafe { (*data).bullet_id }, hex.join("\n"));
            *t = Some((bytes, by_player));
        }
    }
    let orig: FnSpawn = unsafe { std::mem::transmute(SPAWN_ORIGINAL.load(Ordering::Acquire)) };
    unsafe { orig(mgr, out, data_addr, err, a5, a6, a7, a8) }
}

/// Watches the game's own bullet spawns for a SpawnData template (called once at boot).
pub fn install_spawn_hook() {
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    let Ok(exe) = (unsafe { GetModuleHandleW(None) }) else { return };
    let target = (exe.0 as usize + SPAWN_BULLET_RVA) as *mut c_void;
    if let Some(orig) = unsafe { crate::hook::install("SpawnBullet", target, spawn_detour as *mut c_void) } {
        SPAWN_ORIGINAL.store(orig, Ordering::Release);
    }
}

fn spawn(bullet: i64, from: Vec3, dir: Vec3) {
    let (Some(p), Some(mgr)) = (world::player(), world::bullets()) else { return };
    // the mod's own shots stay off until SpawnBullet's real signature is known (see FnSpawn)
    if !SPAWN_SIGNATURE_KNOWN {
        static WARNED_SIG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !WARNED_SIG.swap(true, Ordering::Relaxed) {
            crate::log!("guns: shooting disabled until SpawnBullet's signature is confirmed (see the SpawnBullet lines above)");
        }
        return;
    }
    let Some((template, _)) = *TEMPLATE.lock().unwrap() else {
        static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !WARNED.swap(true, Ordering::Relaxed) {
            crate::log!("guns: no bullet template yet (the game has not spawned a bullet of its own); shot skipped");
        }
        return;
    };
    let mut d: SpawnData = unsafe { std::mem::transmute(template) };
    d.owner = p.chr_ins.field_ins_handle.clone();
    d.target = unsafe { std::mem::transmute([0xFFu8; std::mem::size_of::<eldenring::cs::FieldInsHandle>()]) };
    d.behavior_id = -1;
    d.magic_id = -1;
    d.bullet_id = bullet as i32;
    d.goods_id = -1;
    d.dummy_poly_id = -1;
    d.angle = [dir.x, dir.y, dir.z, 0.0];
    d.acceleration_angle = [dir.x, dir.y, dir.z, 0.0];
    d.position = [from.x, from.y, from.z, 1.0];
    let data = unsafe { &*(&d as *const SpawnData as *const eldenring::cs::BulletSpawnData) };
    OURS.with(|o| o.set(true));
    let r = mgr.spawn_bullet(data);
    OURS.with(|o| o.set(false));
    if let Err(e) = r {
        crate::log!("guns: spawn_bullet {bullet} failed ({e})");
    }
}

/// Simple pseudo-random in [0,1) seeded from the performance counter (CS2 note gotcha 15).
pub fn rand() -> f32 {
    use std::cell::Cell;
    thread_local!(static S: Cell<u64> = Cell::new({
        let mut c = 0i64;
        unsafe { let _ = windows::Win32::System::Performance::QueryPerformanceCounter(&mut c); }
        (c as u64) ^ (std::process::id() as u64).rotate_left(32) | 1
    }));
    S.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x >> 40) as f32 / (1u64 << 24) as f32
    })
}

fn cone(dir: Vec3, half_angle_deg: f32) -> Vec3 {
    if half_angle_deg <= 0.0 {
        return dir;
    }
    let a = half_angle_deg.to_radians() * rand().sqrt();
    let t = rand() * std::f32::consts::TAU;
    let right = Vec3::Y.cross(dir).normalize_or(Vec3::X);
    let up = dir.cross(right);
    (dir * a.cos() + (right * t.cos() + up * t.sin()) * a.sin()).normalize()
}

/// Who the crosshair is on: (index into nearby list, is head) by ray vs capsule.
fn aimed_target(from: Vec3, dir: Vec3, range: f32) -> Option<bool> {
    let wcm = unsafe { <eldenring::cs::WorldChrMan as fromsoftware_shared::FromStatic>::instance_mut() }.ok()?;
    let mut best: Option<(f32, bool)> = None;
    for e in wcm.chr_inses_by_distance.iter().take(48) {
        let chr = unsafe { e.chr_ins.as_ref() };
        if chr.modules.data.hp <= 0 {
            continue;
        }
        let phys = &chr.modules.physics;
        let base = v(&phys.position);
        let (r, h) = (phys.hit_radius.max(0.3), phys.hit_height.max(1.0));
        // closest approach of the ray to the capsule's axis
        let to = base - from;
        let along = to.dot(dir);
        if along < 0.0 || along > range {
            continue;
        }
        let closest = from + dir * along;
        let dy = (closest.y - base.y).clamp(0.0, h);
        let axis = Vec3::new(base.x, base.y + dy, base.z);
        if closest.distance(axis) <= r && closest.y >= base.y - 0.1 && closest.y <= base.y + h + 0.2 {
            let head = closest.y > base.y + h * 0.82;
            if best.map(|(d, _)| along < d).unwrap_or(true) {
                best = Some((along, head));
            }
        }
    }
    best.map(|(_, h)| h)
}

pub fn falloff(row: &WeaponsRow, dist: f32) -> f32 {
    if dist <= row.falloff_start_m || row.range_m <= row.falloff_start_m {
        1.0
    } else {
        let t = ((dist - row.falloff_start_m) / (row.range_m - row.falloff_start_m)).clamp(0.0, 1.0);
        1.0 + (row.falloff_min - 1.0) * t
    }
}

/// Weapon tick: slots, ADS, fire, reload. `muzzle` is the chest of the character.
pub fn tick(st: &mut State, inp: &Frame, dt: f32, muzzle: Vec3, cam_pos: Vec3, cam_fwd: Vec3) {
    st.fire_cooldown = (st.fire_cooldown - dt).max(0.0);
    st.hit_marker = (st.hit_marker - dt).max(0.0);
    for (i, c) in [CONTROLS_SLOT1, CONTROLS_SLOT2, CONTROLS_SLOT3, CONTROLS_SLOT4, CONTROLS_SLOT5].into_iter().enumerate() {
        if inp.pressed(c) && st.saved.slots[i].is_some() {
            st.held = Held::Slot(i);
            st.reload_left = 0.0;
        }
    }
    if inp.pressed(CONTROLS_PICKAXE) {
        st.held = Held::Pickaxe;
        st.reload_left = 0.0;
    }
    let aiming = inp.down(CONTROLS_AIM) && st.held_gun().is_some() && st.reload_left <= 0.0;
    let blend = dt / CAMERA_ADS_BLEND_S_V.max(0.01);
    st.ads = if aiming { (st.ads + blend).min(1.0) } else { (st.ads - blend).max(0.0) };

    let Some(gun) = st.held_gun() else {
        // pickaxe: Fortnite swing (harvest is handled in harvest.rs); also hits enemies
        let pick = &PICKAXE[PICKAXE_DEFAULT];
        if inp.down(CONTROLS_FIRE) && st.fire_cooldown <= 0.0 && st.mode == crate::state::Mode::Combat {
            st.fire_cooldown = pick.swing_s;
            let tier = crate::loot::tier_here();
            if set_shot(pick.damage * REGION_TIERS[tier].damage_mult, 60.0, pick.reach_m, 0.5) {
                spawn(GUN_BULLET.load(Ordering::Relaxed), muzzle, cam_fwd);
            }
            crate::audio::play(pick.fn_swing_sound, None);
            crate::harvest::swing(st, cam_pos, cam_fwd, pick.reach_m + CAMERA_BOOM_LENGTH_M_V);
        }
        return;
    };
    let row = gun.row();

    // reload
    let ammo = row.ammo;
    if st.reload_left > 0.0 {
        st.reload_left -= dt;
        if st.reload_left <= 0.0 {
            let have = st.held_gun().unwrap().mag;
            let want = if row.reload_per_shell { 1 } else { row.mag - have };
            let got = want.min(st.saved.ammo[ammo]);
            st.held_gun_mut().unwrap().mag += got;
            st.saved.ammo[ammo] -= got;
            st.dirty = true;
            let full = st.held_gun().unwrap().mag >= row.mag;
            if row.reload_per_shell && !full && st.saved.ammo[ammo] > 0 {
                st.reload_left = gun.reload_s() / row.mag as f32;
            }
        }
        if !(row.reload_per_shell && inp.pressed(CONTROLS_FIRE) && gun.mag > 0) {
            return;
        }
        st.reload_left = 0.0; // pump: firing interrupts a shell reload
    }
    let start_reload = |st: &mut State| {
        if st.saved.ammo[ammo] > 0 && gun.mag < row.mag {
            st.reload_left = if row.reload_per_shell { gun.reload_s() / row.mag as f32 } else { gun.reload_s() };
            crate::audio::play(row.fn_reload_sound, None);
        }
    };
    if inp.pressed(CONTROLS_RELOAD) {
        start_reload(st);
        return;
    }

    // bloom recovers when not firing
    let base_spread = row.spread_hip_deg + (row.spread_ads_deg - row.spread_hip_deg) * st.ads;
    let (speed, _, sprinting) = crate::movement::speed_now();
    let move_pen = if speed > 0.5 { 1.0 } else { 0.0 } + if sprinting { 1.5 } else { 0.0 };
    st.bloom = (st.bloom - dt * 4.0).max(0.0);

    let trigger = if row.auto { inp.down(CONTROLS_FIRE) } else { inp.pressed(CONTROLS_FIRE) };
    if !trigger || st.fire_cooldown > 0.0 {
        return;
    }
    if gun.mag == 0 {
        start_reload(st);
        return;
    }
    st.fire_cooldown = 1.0 / row.fire_rate;
    st.held_gun_mut().unwrap().mag -= 1;
    st.dirty = true;

    // aim point: what the crosshair is on (camera ray), so shots from the shoulder go where you look
    let far = cam_pos + cam_fwd * row.range_m;
    let aim_point = world::ray(cam_pos, far).unwrap_or(far);
    let dir = (aim_point - muzzle).normalize_or(cam_fwd);
    let spread = base_spread + st.bloom + move_pen * (1.0 - st.ads * 0.7);
    let dist = aim_point.distance(muzzle);
    let head = aimed_target(cam_pos, cam_fwd, row.range_m);
    let mult = if head == Some(true) { row.headshot_mult } else { 1.0 };
    let per_pellet = gun.damage() * falloff(row, dist) * mult / row.pellets as f32;
    if set_shot(per_pellet, row.bullet_speed_mps, row.range_m, 0.08) {
        for _ in 0..row.pellets {
            spawn(GUN_BULLET.load(Ordering::Relaxed), muzzle, cone(dir, spread));
        }
    }
    if row.explosion_radius_m > 0.0 {
        crate::loot::queue_blast(aim_point, dist / row.bullet_speed_mps, gun.damage(), row.explosion_radius_m, row.build_damage_mult);
    }
    if head.is_some() {
        st.hit_marker = 0.25;
        st.damage_numbers.push(([aim_point.x, aim_point.y, aim_point.z], gun.damage() * mult, 1.0, head.unwrap() as u8));
    }
    crate::building::bullet_hits(muzzle, dir, row.range_m, gun.damage() * row.build_damage_mult, st);
    st.bloom = (st.bloom + row.bloom_per_shot_deg).min(row.spread_hip_deg * 2.0);
    st.pitch = (st.pitch + row.recoil_pitch_deg.to_radians() * (1.0 - 0.5 * st.ads)).min(CAMERA_PITCH_MAX_DEG_V.to_radians());
    crate::audio::play(row.fn_fire_sound, None);
}

/// Spawns the rocket explosion bullet at a point (called by loot::tick when the rocket arrives).
pub fn blast(at: Vec3, damage: f32, radius: f32) {
    let (bb, ba) = (BLAST_BULLET.load(Ordering::Relaxed), BLAST_ATK.load(Ordering::Relaxed));
    let Some(params) = world::params() else { return };
    if bb < 0 || ba < 0 {
        return;
    }
    if let Some(a) = params.get_mut::<AtkParam_Pc>(ba as u32) {
        a.set_atk_phys(damage.round().clamp(1.0, 65535.0) as u16);
        a.set_hit0_radius(radius);
    }
    if let Some(b) = params.get_mut::<Bullet>(bb as u32) {
        b.set_hit_radius(radius);
    }
    spawn(bb, at, Vec3::Y);
}

pub fn muzzle_point(feet: Vec3, yaw: f32) -> Vec3 {
    feet + Vec3::Y * 1.45 + camera::flat_right(yaw) * 0.25 + camera::flat_forward(yaw) * 0.4
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn falloff_is_one_inside_start_and_min_at_range() {
        let ar = &WEAPONS[WEAPONS_AR];
        assert_eq!(falloff(ar, 10.0), 1.0);
        assert!((falloff(ar, ar.range_m) - ar.falloff_min).abs() < 1e-5);
    }
}
