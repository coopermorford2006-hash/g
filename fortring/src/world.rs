//! Access to Elden Ring's live objects (fromsoftware-rs). Every accessor returns None until the
//! object exists, so systems simply skip a frame on the title screen or during loading.
use eldenring::cs::{
    CSBulletManager, CSCamera, CSFeManImp, CSHavokMan, CSMenuManImp, ChrIns, PlayerIns, SoloParamRepository,
    WorldChrMan,
};
use eldenring::position::{HavokPosition, PositionDelta};
use fromsoftware_shared::FromStatic;
use glam::Vec3;

pub fn player() -> Option<&'static mut PlayerIns> {
    unsafe { WorldChrMan::instance_mut() }.ok()?.main_player.as_deref_mut()
}

pub fn chr(player: &mut PlayerIns) -> &mut ChrIns {
    &mut player.chr_ins
}

pub fn camera() -> Option<&'static mut CSCamera> {
    unsafe { CSCamera::instance_mut() }.ok()
}

pub fn havok() -> Option<&'static mut CSHavokMan> {
    unsafe { CSHavokMan::instance_mut() }.ok()
}

pub fn bullets() -> Option<&'static mut CSBulletManager> {
    unsafe { CSBulletManager::instance_mut() }.ok()
}

pub fn params() -> Option<&'static mut SoloParamRepository> {
    unsafe { SoloParamRepository::instance_mut() }.ok()
}

pub fn fe_man() -> Option<&'static mut CSFeManImp> {
    unsafe { CSFeManImp::instance_mut() }.ok()
}

pub fn menu_man() -> Option<&'static mut CSMenuManImp> {
    unsafe { CSMenuManImp::instance_mut() }.ok()
}

pub fn v(p: &HavokPosition) -> Vec3 {
    Vec3::new(p.0, p.1, p.2)
}

pub fn hp(v: Vec3) -> HavokPosition {
    HavokPosition(v.x, v.y, v.z, 0.0)
}

/// Havok filter that hits map collision (CS2 conversion note: 0x2000058).
pub const MAP_FILTER: u32 = 0x0200_0058;

/// Build pieces in the player's tile (Havok space), refreshed each tick by building::refresh.
pub static BUILD_BOXES: std::sync::Mutex<Vec<crate::building::Obb>> = std::sync::Mutex::new(Vec::new());

/// Ray cast against the map and the player's builds; returns the nearest hit point.
pub fn ray(from: Vec3, to: Vec3) -> Option<Vec3> {
    let map = map_ray(from, to);
    let dir = (to - from).normalize_or_zero();
    let build = BUILD_BOXES.lock().unwrap().iter().filter_map(|b| b.ray(from, to)).min_by(|a, b| a.total_cmp(b)).map(|t| from + dir * t);
    match (map, build) {
        (Some(m), Some(b)) => Some(if m.distance(from) < b.distance(from) { m } else { b }),
        (m, b) => m.or(b),
    }
}

/// Ray cast against the map only.
pub fn map_ray(from: Vec3, to: Vec3) -> Option<Vec3> {
    let player = player()?;
    let world = havok()?;
    let d = to - from;
    world.phys_world.cast_ray(MAP_FILTER, &hp(from), PositionDelta(d.x, d.y, d.z), player).map(|p| v(&p))
}

/// HP and max HP of a character by its field-ins handle (boss bars).
pub fn hp_of(handle: &eldenring::cs::FieldInsHandle) -> Option<(i32, i32)> {
    let wcm = unsafe { WorldChrMan::instance_mut() }.ok()?;
    for e in wcm.chr_inses_by_distance.iter() {
        let chr = unsafe { e.chr_ins.as_ref() };
        if chr.field_ins_handle.selector == handle.selector && chr.field_ins_handle.block_id == handle.block_id {
            return Some((chr.modules.data.hp, chr.modules.data.max_hp));
        }
    }
    None
}
