//! systems:containers + pickups + inventory rules. Chests and ammo boxes sit around every Site of Grace
//! (spawn_rules), rolls come from loot_tables with rarity weights from the region tier, and loot pops
//! out as floating pickups like in Fortnite.
use crate::generated::*;
use crate::input::Frame;
use crate::state::{Gun, Pickup, State};
use crate::world::{self, v};
use eldenring::cs::BonfireWarpParam;
use glam::Vec3;
use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Clone, Copy, Debug)]
pub struct Grace {
    pub id: u32,
    pub area: u8,
    pub gx: u8,
    pub gz: u8,
    pub local: Vec3,
}

#[derive(Clone, Debug)]
pub struct Container {
    pub key: String,
    pub kind: usize,
    pub pos: Vec3,
    pub tier: usize,
}

struct World {
    graces: Vec<Grace>,
    /// key -> resolved Havok position (None = tile not loaded yet / no ground)
    placed: BTreeMap<String, Option<Vec3>>,
    pub near: Vec<Container>,
    blasts: Vec<(Vec3, f32, f32, f32, f32)>,
    search: Option<(String, f32)>,
}

static WORLD: Mutex<World> = Mutex::new(World { graces: Vec::new(), placed: BTreeMap::new(), near: Vec::new(), blasts: Vec::new(), search: None });

pub fn load_graces() {
    let Some(params) = world::params() else { return };
    let mut w = WORLD.lock().unwrap();
    w.graces = params
        .rows::<BonfireWarpParam>()
        .filter(|(_, r)| r.bonfire_entity_id() != 0 && r.area_no() != 0)
        .map(|(id, r)| Grace { id, area: r.area_no(), gx: r.grid_x_no(), gz: r.grid_z_no(), local: Vec3::new(r.pos_x(), r.pos_y(), r.pos_z()) })
        .collect();
    crate::log!("loot: {} Sites of Grace anchor containers", w.graces.len());
}

/// Region tier for a map tile (area, block, region) from region_tiers.map_areas.
pub fn tier_for(area: u8, block: u8, region: u8) -> usize {
    for (i, t) in REGION_TIERS.iter().enumerate() {
        for m in t.map_areas {
            if matches_area(m, area, block, region) {
                return i;
            }
        }
    }
    0
}

pub fn matches_area(m: &str, area: u8, block: u8, region: u8) -> bool {
    let Some(rest) = m.strip_prefix('m') else { return false };
    let mut parts = rest.split('_');
    let Some(a) = parts.next().and_then(|s| s.parse::<u8>().ok()) else { return false };
    if a != area {
        return false;
    }
    let range = |s: Option<&str>, x: u8| -> bool {
        match s {
            None => true,
            Some(s) => match s.split_once('-') {
                Some((lo, hi)) => matches!((lo.parse::<u8>(), hi.parse::<u8>()), (Ok(lo), Ok(hi)) if (lo..=hi).contains(&x)),
                None => s.parse::<u8>().map(|v| v == x).unwrap_or(false),
            },
        }
    };
    range(parts.next(), block) && range(parts.next(), region)
}

pub fn tier_here() -> usize {
    world::player().map(|p| {
        let b = &p.chr_ins.block_id;
        tier_for(b.area(), b.block(), b.region())
    }).unwrap_or(0)
}

/// Deterministic 0..1 from a key (so a container exists or not the same way every session).
fn hash01(s: &str) -> f32 {
    let mut h: u32 = 2166136261;
    for b in s.bytes() {
        h = (h ^ b as u32).wrapping_mul(16777619);
    }
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Havok position of a block-local point, relative to the player's own tile (open world tiles are 256 m).
fn to_havok(area: u8, gx: u8, gz: u8, local: Vec3) -> Option<Vec3> {
    let p = world::player()?;
    let b = &p.chr_ins.block_id;
    let me = v(&p.chr_ins.modules.physics.position);
    let bp = &p.block_position;
    let mine = Vec3::new(bp.x, bp.y, bp.z);
    if area != b.area() {
        return None;
    }
    let tile = if area == 60 || area == 61 {
        Vec3::new((gx as f32 - b.block() as f32) * 256.0, 0.0, (gz as f32 - b.region() as f32) * 256.0)
    } else if gx == b.block() && gz == b.region() {
        Vec3::ZERO
    } else {
        return None;
    };
    Some(me + (local + tile - mine))
}

pub fn tick(st: &mut State, inp: &Frame, dt: f32, cam_pos: Vec3, cam_fwd: Vec3) {
    let Some(p) = world::player() else { return };
    let me = v(&p.chr_ins.modules.physics.position);
    let mut w = WORLD.lock().unwrap();

    // 1. resolve containers around graces within 200 m
    let graces = w.graces.clone();
    let mut near = Vec::new();
    for g in graces.iter() {
        let Some(gpos) = to_havok(g.area, g.gx, g.gz, g.local) else { continue };
        if gpos.distance(me) > 200.0 {
            continue;
        }
        let tier = tier_for(g.area, g.gx, g.gz);
        for (kind, rule) in [(CONTAINERS_CHEST, SPAWN_RULES_CHEST_RULE), (CONTAINERS_AMMO_BOX, SPAWN_RULES_AMMO_RULE)] {
            let rule = &SPAWN_RULES[rule];
            for i in 0..rule.count {
                let key = format!("{}:{}:{}", g.id, CONTAINERS[kind].id, i);
                if hash01(&key) > rule.chance {
                    continue;
                }
                if rule.respawn == "never" && st.saved.opened.contains(&key) {
                    continue;
                }
                let placed = w.placed.entry(key.clone()).or_insert(None);
                if placed.is_none() {
                    let ang = hash01(&(key.clone() + "a")) * std::f32::consts::TAU;
                    let r = rule.offset_m * (0.6 + 0.4 * hash01(&(key.clone() + "r")));
                    let guess = gpos + Vec3::new(ang.cos() * r, 0.0, ang.sin() * r);
                    *placed = world::ray(guess + Vec3::Y * 20.0, guess - Vec3::Y * 40.0);
                }
                if let Some(pos) = *placed {
                    near.push(Container { key, kind, pos, tier });
                }
            }
        }
    }
    // keep spacing: drop containers too close to an earlier one
    let mut kept: Vec<Container> = Vec::new();
    for c in near {
        let spacing = SPAWN_RULES[CONTAINERS[c.kind].placement].min_spacing_m;
        if kept.iter().all(|k| k.pos.distance(c.pos) >= spacing) {
            kept.push(c);
        }
    }
    w.near = kept;

    // 2. interaction: the closest container or pickup in front of the player
    st.prompt = None;
    let looking = |pos: Vec3| (pos - cam_pos).normalize_or_zero().dot(cam_fwd) > 0.75;
    let target = w.near.iter().filter(|c| c.pos.distance(me) < 2.5 && looking(c.pos + Vec3::Y * 0.5))
        .min_by(|a, b| a.pos.distance(me).total_cmp(&b.pos.distance(me))).cloned();
    if let Some(c) = target {
        let row = &CONTAINERS[c.kind];
        st.prompt = Some(format!("[E] {}", row.name));
        st.prompt_color = [1.0, 0.85, 0.3, 1.0];
        if inp.down(CONTROLS_INTERACT) {
            let t = match &w.search { Some((k, t)) if *k == c.key => t + dt, _ => dt };
            w.search = Some((c.key.clone(), t));
            st.progress = Some(("Searching".into(), (t / row.open_s).min(1.0)));
            if t >= row.open_s {
                w.search = None;
                st.progress = None;
                open(st, &c);
            }
        } else if w.search.is_some() {
            w.search = None;
            st.progress = None;
        }
    } else {
        pickups(st, inp, me, &looking);
    }

    // 3. rockets in flight explode on arrival
    let mut i = 0;
    while i < w.blasts.len() {
        w.blasts[i].1 -= dt;
        if w.blasts[i].1 <= 0.0 {
            let (at, _, dmg, radius, build_mult) = w.blasts.remove(i);
            crate::guns::blast(at, dmg, radius);
            crate::building::blast(at, radius, dmg * build_mult, st);
            crate::audio::play(FORTNITE_ASSETS_SND_ROCKET_FIRE, Some(at));
        } else {
            i += 1;
        }
    }
}

pub fn queue_blast(at: Vec3, delay: f32, damage: f32, radius: f32, build_mult: f32) {
    WORLD.lock().unwrap().blasts.push((at, delay, damage, radius, build_mult));
}

pub fn near_containers() -> Vec<Container> {
    WORLD.lock().unwrap().near.clone()
}

fn open(st: &mut State, c: &Container) {
    let row = &CONTAINERS[c.kind];
    st.saved.opened.push(c.key.clone());
    st.dirty = true;
    crate::audio::play(row.fn_open_sound, Some(c.pos));
    let table = &LOOT_TABLES[row.loot];
    let mut out = Vec::new();
    for _ in 0..row.rolls {
        out.push(roll(table.entries, c.tier, c.kind == CONTAINERS_AMMO_BOX));
    }
    crate::log!("loot: opened {} (tier {}) -> {:?}", c.key, REGION_TIERS[c.tier].id, out.iter().map(|p| p.item).collect::<Vec<_>>());
    for (i, mut p) in out.into_iter().enumerate() {
        let a = i as f32 * 2.1;
        let at = c.pos + Vec3::new(a.cos() * 1.0, 0.4, a.sin() * 1.0);
        p.pos = [at.x, at.y, at.z];
        // a gun brings a magazine's worth of its ammo, like Fortnite
        if let Some(g) = p.gun {
            st.pickups.push(Pickup { item: Ref::Ammo(g.row().ammo), gun: None, count: AMMO[g.row().ammo].chest_drop, pos: [at.x + 0.5, at.y, at.z] });
        }
        st.pickups.push(p);
    }
}

/// One weighted roll from a loot table.
pub fn roll(entries: &[(Ref, f32, u32)], tier: usize, ammo_box: bool) -> Pickup {
    let total: f32 = entries.iter().map(|e| e.1).sum();
    let mut x = crate::guns::rand() * total;
    let mut pick = entries[0];
    for e in entries {
        if x < e.1 {
            pick = *e;
            break;
        }
        x -= e.1;
    }
    let (item, _, count) = pick;
    match item {
        Ref::Weapons(w) => {
            let rarity = roll_rarity(tier, w);
            Pickup { item, gun: Some(Gun { weapon: w, rarity, tier, mag: WEAPONS[w].mag }), count: 1, pos: [0.0; 3] }
        }
        Ref::Ammo(a) => {
            let n = if count > 0 { count } else if ammo_box { AMMO[a].box_drop } else { AMMO[a].chest_drop };
            Pickup { item, gun: None, count: n, pos: [0.0; 3] }
        }
        _ => Pickup { item, gun: None, count: count.max(1), pos: [0.0; 3] },
    }
}

fn roll_rarity(tier: usize, weapon: usize) -> usize {
    let weights = REGION_TIERS[tier].rarity_weights;
    let allowed = WEAPONS[weapon].rarities;
    let total: f32 = allowed.iter().map(|&r| weights[RARITIES[r].index as usize]).sum();
    let mut x = crate::guns::rand() * total.max(1e-6);
    for &r in allowed {
        let wgt = weights[RARITIES[r].index as usize];
        if x < wgt {
            return r;
        }
        x -= wgt;
    }
    allowed[0]
}

fn pickups(st: &mut State, inp: &Frame, me: Vec3, looking: &dyn Fn(Vec3) -> bool) {
    let idx = st.pickups.iter().enumerate()
        .filter(|(_, p)| Vec3::from(p.pos).distance(me) < 2.2 && looking(Vec3::from(p.pos)))
        .min_by(|a, b| Vec3::from(a.1.pos).distance(me).total_cmp(&Vec3::from(b.1.pos).distance(me)))
        .map(|(i, _)| i);
    // auto-pickup ammo, materials and stackables you walk over (Fortnite)
    let mut i = 0;
    while i < st.pickups.len() {
        let p = st.pickups[i].clone();
        if p.gun.is_none() && Vec3::from(p.pos).distance(me) < 1.2 && add_stack(st, p.item, p.count) == 0 {
            st.pickups.remove(i);
        } else {
            i += 1;
        }
    }
    let Some(i) = idx else { return };
    let Some(p) = st.pickups.get(i).cloned() else { return };
    let (name, color) = describe(&p);
    st.prompt = Some(format!("[E] Pick up {name}"));
    st.prompt_color = color;
    if !inp.pressed(CONTROLS_INTERACT) {
        return;
    }
    if let Some(g) = p.gun {
        let slot = st.saved.slots.iter().position(|s| s.is_none());
        match slot {
            Some(s) => {
                st.saved.slots[s] = Some(g);
                st.held = crate::state::Held::Slot(s);
                st.pickups.remove(i);
            }
            None => {
                // full: swap with the held gun (or slot 1 when holding the pickaxe)
                let s = match st.held { crate::state::Held::Slot(s) => s, _ => 0 };
                let old = st.saved.slots[s].replace(g);
                st.pickups[i].gun = old;
            }
        }
    } else {
        let left = add_stack(st, p.item, p.count);
        if left == 0 { st.pickups.remove(i); } else { st.pickups[i].count = left; }
    }
    st.dirty = true;
}

/// Adds stackable loot; returns how much didn't fit.
pub fn add_stack(st: &mut State, item: Ref, count: u32) -> u32 {
    let (have, max) = match item {
        Ref::Ammo(a) => (&mut st.saved.ammo[a], AMMO[a].max),
        Ref::Materials(m) => (&mut st.saved.materials[m], MATERIALS[m].max),
        Ref::Consumables(c) => (&mut st.saved.consumables[c], CONSUMABLES[c].stack),
        _ => return count,
    };
    let space = max.saturating_sub(*have);
    let took = space.min(count);
    *have += took;
    count - took
}

pub fn describe(p: &Pickup) -> (String, [f32; 4]) {
    let hex = hex_color;
    match (p.item, p.gun) {
        (_, Some(g)) => (format!("{} {}", RARITIES[g.rarity].name, g.row().name), hex(RARITIES[g.rarity].color)),
        (Ref::Ammo(a), _) => (format!("{} x{}", AMMO[a].name, p.count), [1.0; 4]),
        (Ref::Materials(m), _) => (format!("{} x{}", MATERIALS[m].name, p.count), [1.0; 4]),
        (Ref::Consumables(c), _) => (format!("{} x{}", CONSUMABLES[c].name, p.count), [1.0; 4]),
        _ => ("item".into(), [1.0; 4]),
    }
}

pub fn hex_color(h: &str) -> [f32; 4] {
    let n = u32::from_str_radix(h.trim_start_matches('#'), 16).unwrap_or(0xFFFFFF);
    [((n >> 16) & 255) as f32 / 255.0, ((n >> 8) & 255) as f32 / 255.0, (n & 255) as f32 / 255.0, 1.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn area_patterns() {
        assert!(matches_area("m10", 10, 0, 0));
        assert!(matches_area("m31_09", 31, 9, 0));
        assert!(!matches_area("m31_09", 31, 10, 0));
        assert!(matches_area("m60_40-46_32-41", 60, 42, 36));
        assert!(!matches_area("m60_40-46_32-41", 60, 47, 36));
    }
    #[test]
    fn church_of_elleh_is_tier_one() {
        // m60_42_36 is Limgrave's opening area
        assert_eq!(tier_for(60, 42, 36), REGION_TIERS_T1);
    }
    #[test]
    fn rolls_always_give_something_valid() {
        for t in 0..REGION_TIERS.len() {
            for _ in 0..500 {
                let p = roll(LOOT_TABLES[LOOT_TABLES_CHEST].entries, t, false);
                if let Some(g) = p.gun {
                    assert!(WEAPONS[g.weapon].rarities.contains(&g.rarity));
                    assert!(REGION_TIERS[t].rarity_weights[RARITIES[g.rarity].index as usize] > 0.0, "tier {t} rolled a zero-weight rarity");
                }
                assert!(p.count > 0);
            }
        }
    }
    #[test]
    fn stacks_cap() {
        let mut st = crate::state::State::new_for_tests();
        assert_eq!(add_stack(&mut st, Ref::Materials(MATERIALS_WOOD), 2000), 2000 - MATERIALS[MATERIALS_WOOD].max);
    }
}
