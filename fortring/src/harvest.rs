//! systems:harvest. A pickaxe swing ray-casts the world; the nearest placed map asset (MSB part) to
//! the hit decides the material by its name prefix (harvestables.matches). Its MSB part position is
//! read from the loaded map file image (MSBE part: Position at +0x20 after the name pointer, ids and
//! sibling offset, per SoulsFormats' layout).
use crate::generated::*;
use crate::state::State;
use crate::world::{self, v};
use eldenring::cs::CSWorldGeomMan;
use fromsoftware_shared::FromStatic;
use glam::Vec3;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

const MSB_PART_POSITION: usize = 0x20;

struct Node {
    hits: u32,
    exhausted_at: Option<Instant>,
}
static NODES: Mutex<Option<HashMap<String, Node>>> = Mutex::new(None);

fn part_name(ptr: *const u16) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    let mut out = String::new();
    for i in 0..64 {
        let c = unsafe { *ptr.add(i) };
        if c == 0 {
            return Some(out);
        }
        out.push(char::from_u32(c as u32)?);
    }
    None
}

/// The closest map asset to a block-local point: (name, distance).
fn nearest_asset(local: Vec3, max: f32) -> Option<(String, f32)> {
    let p = world::player()?;
    let gm = unsafe { CSWorldGeomMan::instance_mut() }.ok()?;
    let block = gm.geom_block_data_by_id(&p.chr_ins.block_id)?;
    let mut best: Option<(String, f32)> = None;
    for ins in block.geom_ins_vector.iter() {
        let part = ins.info.msb_parts_geom.msb_parts.msb_part.as_ptr();
        if part.is_null() {
            continue;
        }
        let name_ptr = unsafe { (*part).name.0 };
        let Some(name) = part_name(name_ptr) else { continue };
        let pos = unsafe { *((part as *const u8).add(MSB_PART_POSITION) as *const [f32; 3]) };
        let d = Vec3::from(pos).distance(local);
        if d < max && best.as_ref().map(|b| d < b.1).unwrap_or(true) {
            best = Some((name, d));
        }
    }
    best
}

pub fn classify(name: &str) -> Option<usize> {
    HARVESTABLES.iter().position(|h| h.matches.iter().any(|m| !m.starts_with("material:") && name.starts_with(m)))
}

pub fn swing(st: &mut State, cam_pos: Vec3, cam_fwd: Vec3, reach: f32) {
    let Some(p) = world::player() else { return };
    let me = v(&p.chr_ins.modules.physics.position);
    let Some(hit) = world::ray(cam_pos, cam_pos + cam_fwd * reach) else { return };
    if hit.distance(me + Vec3::Y) > PICKAXE[PICKAXE_DEFAULT].reach_m + 0.8 {
        return;
    }
    // a build piece in front takes the hit first (Fortnite: you can farm your own builds)
    if crate::building::pickaxe_hit(hit, st) {
        return;
    }
    let bp = &p.block_position;
    let local = Vec3::new(bp.x, bp.y, bp.z) + (hit - me);
    let found = nearest_asset(local, 4.0);
    let row = found.as_ref().and_then(|(n, _)| classify(n)).or_else(|| {
        // bare terrain and ruins: the row whose matches include map material stone
        HARVESTABLES.iter().position(|h| h.matches.iter().any(|m| *m == "material:stone_brick"))
    });
    crate::log!("harvest: hit {:?} -> {}", found, row.map(|r| HARVESTABLES[r].id).unwrap_or("nothing"));
    let Some(row) = row else { return };
    let h = &HARVESTABLES[row];
    let key = found.map(|(n, _)| n).unwrap_or_else(|| format!("terrain:{:.0}:{:.0}", local.x, local.z));
    let mut nodes = NODES.lock().unwrap();
    let node = nodes.get_or_insert_with(HashMap::new).entry(key).or_insert(Node { hits: 0, exhausted_at: None });
    if let Some(t) = node.exhausted_at {
        if t.elapsed().as_secs_f32() < h.respawn_s {
            return;
        }
        node.exhausted_at = None;
        node.hits = 0;
    }
    node.hits += 1;
    if h.hits_to_break > 0 && node.hits >= h.hits_to_break {
        node.exhausted_at = Some(Instant::now());
    }
    // Fortnite's weak point: every third swing on the same node lands on it
    let crit = node.hits % 3 == 0;
    let amount = if crit { h.crit_per_hit } else { h.per_hit };
    let left = crate::loot::add_stack(st, Ref::Materials(h.material), amount);
    st.damage_numbers.push(([hit.x, hit.y, hit.z], (amount - left) as f32, 1.0, if crit { 2 } else { 3 }));
    st.dirty = true;
    crate::audio::play(PICKAXE[PICKAXE_DEFAULT].fn_hit_sound, Some(hit));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn names_classify_by_prefix() {
        assert_eq!(classify("AEG463_120_9000").map(|r| HARVESTABLES[r].id), Some("trees"));
        assert_eq!(classify("AEG220_005_1000").map(|r| HARVESTABLES[r].id), Some("rocks"));
        assert_eq!(classify("m60_42_36_00"), None);
    }
}
