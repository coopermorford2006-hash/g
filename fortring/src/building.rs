//! systems:building. Fortnite building on Fortnite's grid. Pieces are oriented boxes in block-local
//! space (state::Build). world::ray includes them, so the movement controller, camera and guns all
//! collide with builds.
use crate::camera::flat_forward;
use crate::generated::*;
use crate::input::Frame;
use crate::state::{Build, Mode, State};
use crate::world::{self, v};
use glam::{Mat3, Vec3};

pub const TILE: f32 = 5.12;
pub const STOREY: f32 = 3.84;

#[derive(Clone, Copy, Debug)]
pub struct Obb {
    pub center: Vec3,
    pub axes: Mat3,
    pub half: Vec3,
}

impl Obb {
    pub fn of(piece: usize, center: Vec3, yaw: f32) -> Obb {
        let row = &BUILD_PIECES[piece];
        let axes = Mat3::from_rotation_y(yaw) * Mat3::from_rotation_x(-row.pitch_deg.to_radians());
        let [x, y, z] = row.size_m;
        // sheet sizes are [width, depth, height]; box local axes are x = width, y = height, z = depth
        Obb { center, axes, half: Vec3::new(x, z, y) * 0.5 }
    }

    /// Ray (segment) vs box: distance along the segment's direction to the entry point.
    pub fn ray(&self, from: Vec3, to: Vec3) -> Option<f32> {
        let d = to - from;
        let len = d.length();
        if len < 1e-6 {
            return None;
        }
        let inv = self.axes.transpose();
        let o = inv * (from - self.center);
        let dir = inv * (d / len);
        let (mut tmin, mut tmax) = (0.0f32, len);
        for i in 0..3 {
            let (oi, di, hi) = (o[i], dir[i], self.half[i]);
            if di.abs() < 1e-8 {
                if oi.abs() > hi {
                    return None;
                }
            } else {
                let (mut t1, mut t2) = ((-hi - oi) / di, (hi - oi) / di);
                if t1 > t2 {
                    std::mem::swap(&mut t1, &mut t2);
                }
                tmin = tmin.max(t1);
                tmax = tmax.min(t2);
                if tmin > tmax {
                    return None;
                }
            }
        }
        Some(tmin)
    }
}

/// Block-local <-> Havok for the player's current tile.
fn local_to_havok(local: Vec3) -> Option<Vec3> {
    let p = world::player()?;
    let bp = &p.block_position;
    Some(v(&p.chr_ins.modules.physics.position) + (local - Vec3::new(bp.x, bp.y, bp.z)))
}

fn havok_to_local(h: Vec3) -> Option<Vec3> {
    let p = world::player()?;
    let bp = &p.block_position;
    Some(Vec3::new(bp.x, bp.y, bp.z) + (h - v(&p.chr_ins.modules.physics.position)))
}

fn block_key() -> u32 {
    world::player().map(|p| {
        let b = &p.chr_ins.block_id;
        (b.area() as u32) << 24 | (b.block() as u32) << 16 | (b.region() as u32) << 8 | b.index() as u32
    }).unwrap_or(0)
}

/// Boxes of the builds in the player's tile, in Havok space.
pub fn boxes(st: &State) -> Vec<(usize, Obb)> {
    let here = block_key();
    st.saved.builds.iter().enumerate().filter(|(_, b)| b.block == here)
        .filter_map(|(i, b)| local_to_havok(Vec3::from(b.pos)).map(|c| (i, Obb::of(b.piece, c, b.yaw))))
        .collect()
}

/// Snapped placement for the selected piece in front of the player: (block-local centre, yaw).
pub fn preview(st: &State, cam_pos: Vec3, cam_fwd: Vec3) -> Option<(Vec3, f32)> {
    let p = world::player()?;
    let me = v(&p.chr_ins.modules.physics.position);
    // aim point: what the crosshair hits within reach, else a point 4 m ahead
    let reach = 4.0 + crate::generated::CAMERA_BOOM_LENGTH_M_V;
    let aim = world::ray(cam_pos, cam_pos + cam_fwd * reach).unwrap_or(cam_pos + cam_fwd * reach);
    let target = if aim.distance(me) > TILE * 1.2 { me + (aim - me).normalize_or_zero() * TILE * 0.6 } else { aim };
    let local = havok_to_local(target)?;
    let feet_local = havok_to_local(me)?;
    // the player's facing snapped to 90 degrees picks which edge/direction
    let quarter = (st.yaw / std::f32::consts::FRAC_PI_2).round();
    let yaw = quarter * std::f32::consts::FRAC_PI_2 + st.build_rotation as f32 * std::f32::consts::FRAC_PI_2;
    let cell = |x: f32| (x / TILE).floor() * TILE + TILE * 0.5;
    let floor_y = ((feet_local.y + 0.3) / STOREY).floor() * STOREY;
    let (cx, cz) = (cell(local.x), cell(local.z));
    let row = &BUILD_PIECES[st.build_piece];
    let pos = match row.pivot {
        "wall_edge" => {
            let f = flat_forward(quarter * std::f32::consts::FRAC_PI_2);
            Vec3::new(cell(feet_local.x) + f.x * TILE * 0.5, floor_y + STOREY * 0.5, cell(feet_local.z) + f.z * TILE * 0.5)
        }
        "floor_cell" => Vec3::new(cx, floor_y + if local.y > floor_y + STOREY * 0.6 { STOREY } else { 0.0 }, cz),
        "cone_cell" => Vec3::new(cx, floor_y + row.size_m[2] * 0.5, cz),
        _ => Vec3::new(cx, floor_y + STOREY * 0.5, cz), // ramp_cell
    };
    Some((pos, yaw))
}

/// Publishes this tile's build boxes for world::ray (collision for movement, camera and guns), and what
/// the HUD draws: every piece here plus the placement preview in build mode.
pub fn refresh(st: &mut State, cam_pos: Vec3, cam_fwd: Vec3) {
    let here = boxes(st);
    *world::BUILD_BOXES.lock().unwrap() = here.iter().map(|(_, b)| b.clone()).collect();
    // pieces hidden behind terrain are not drawn (centre and the box's corners tested)
    let mut draw: Vec<(usize, usize, [f32; 3], f32, bool)> = here.iter()
        .filter(|(_, b)| b.center.distance(cam_pos) < 150.0)
        .filter(|(_, b)| {
            let mut pts = vec![b.center];
            // corners across the box's two largest extents (a wall's width and height, a floor's width and depth)
            let h = b.half.to_array();
            let mut order = [0usize, 1, 2];
            order.sort_by(|a, c| h[*c].total_cmp(&h[*a]));
            for (sa, sb) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                let mut l = [0.0f32; 3];
                l[order[0]] = h[order[0]] * sa * 0.9;
                l[order[1]] = h[order[1]] * sb * 0.9;
                pts.push(b.center + b.axes * Vec3::from(l));
            }
            world::visible(cam_pos, &pts)
        })
        .map(|(i, b)| { let p = &st.saved.builds[*i]; (p.piece, p.material, b.center.into(), p.yaw, false) })
        .collect();
    if st.mode == crate::state::Mode::Build {
        if let Some((local, yaw)) = preview(st, cam_pos, cam_fwd) {
            if let Some(c) = local_to_havok(local) {
                draw.push((st.build_piece, st.build_material, c.into(), yaw, true));
            }
        }
    }
    st.build_draw = draw;
}

pub fn tick(st: &mut State, inp: &Frame, dt: f32, now: f64, cam_pos: Vec3, cam_fwd: Vec3) {
    // HP grows while a piece builds up (Fortnite: starts at start_hp_frac of max over build_s)
    for b in st.saved.builds.iter_mut() {
        let m = &MATERIALS[b.material];
        let age = (now - b.placed_at) as f32;
        if age < m.build_s {
            b.hp = (b.hp + b.max_hp * (1.0 - m.start_hp_frac) * dt / m.build_s).min(b.max_hp);
        }
    }
    let was = st.mode;
    if inp.pressed(CONTROLS_BUILD_MODE) {
        st.mode = if st.mode == Mode::Build { Mode::Combat } else { Mode::Build };
    }
    for (c, piece) in [(CONTROLS_BUILD_WALL, BUILD_PIECES_WALL), (CONTROLS_BUILD_FLOOR, BUILD_PIECES_FLOOR), (CONTROLS_BUILD_RAMP, BUILD_PIECES_RAMP), (CONTROLS_BUILD_CONE, BUILD_PIECES_CONE)] {
        if inp.pressed(c) {
            st.mode = Mode::Build;
            st.build_piece = piece;
        }
    }
    if st.mode != was {
        st.build_rotation = 0;
        crate::log!("building: mode {:?}", st.mode);
    }
    if st.mode != Mode::Build {
        return;
    }
    if inp.pressed(CONTROLS_BUILD_MATERIAL) {
        st.build_material = (st.build_material + 1) % MATERIALS.len();
    }
    // no rotation key: pieces face where the player looks, like Fortnite's defaults
    if !inp.down(CONTROLS_FIRE) || st.fire_cooldown > 0.0 {
        return;
    }
    let Some((pos, yaw)) = preview(st, cam_pos, cam_fwd) else { return };
    let here = block_key();
    if st.saved.builds.iter().any(|b| b.block == here && b.piece == st.build_piece && Vec3::from(b.pos).distance(pos) < 0.5 && (b.yaw - yaw).abs() < 0.1) {
        return; // already a piece there
    }
    let m = &MATERIALS[st.build_material];
    if st.saved.materials[st.build_material] < m.piece_cost {
        st.message(format!("Not enough {}", m.name));
        st.fire_cooldown = 0.3;
        return;
    }
    st.saved.materials[st.build_material] -= m.piece_cost;
    let max_hp = match BUILD_PIECES[st.build_piece].hp_column {
        "wall_hp" => m.wall_hp,
        "floor_hp" => m.floor_hp,
        _ => m.ramp_hp,
    } as f32;
    st.saved.builds.push(Build { piece: st.build_piece, material: st.build_material, pos: pos.into(), yaw, hp: max_hp * m.start_hp_frac, max_hp, placed_at: now, block: here });
    st.fire_cooldown = 0.05; // Fortnite's build rate
    st.dirty = true;
    crate::log!("building: placed {} {} at {pos} ({} left)", m.name, BUILD_PIECES[st.build_piece].name, st.saved.materials[st.build_material]);
}

fn damage(st: &mut State, idx: usize, amount: f32) {
    if let Some(b) = st.saved.builds.get_mut(idx) {
        b.hp -= amount;
        if b.hp <= 0.0 {
            crate::log!("building: {} destroyed", BUILD_PIECES[b.piece].name);
            st.saved.builds.remove(idx);
        }
        st.dirty = true;
    }
}

/// First build the segment hits: (build index, distance).
pub fn ray_builds(st: &State, from: Vec3, to: Vec3) -> Option<(usize, f32)> {
    boxes(st).into_iter().filter_map(|(i, b)| b.ray(from, to).map(|t| (i, t))).min_by(|a, b| a.1.total_cmp(&b.1))
}

pub fn bullet_hits(from: Vec3, dir: Vec3, range: f32, dmg: f32, st: &mut State) {
    let map = world::map_ray(from, from + dir * range).map(|h| h.distance(from)).unwrap_or(f32::MAX);
    if let Some((i, t)) = ray_builds(st, from, from + dir * range) {
        if t < map {
            damage(st, i, dmg);
        }
    }
}

pub fn blast(at: Vec3, radius: f32, dmg: f32, st: &mut State) {
    let mut hit: Vec<usize> = boxes(st).into_iter().filter(|(_, b)| b.center.distance(at) < radius + b.half.max_element()).map(|(i, _)| i).collect();
    hit.sort_unstable_by(|a, b| b.cmp(a));
    for i in hit {
        damage(st, i, dmg);
    }
}

/// Pickaxe on a build: Fortnite deals 50 to structures per swing.
pub fn pickaxe_hit(at: Vec3, st: &mut State) -> bool {
    let near = boxes(st).into_iter().find(|(_, b)| b.ray(at - Vec3::Y * 0.01, at + Vec3::Y * 0.01).is_some()
        || (b.axes.transpose() * (at - b.center)).abs().cmple(b.half + Vec3::splat(0.15)).all());
    if let Some((i, _)) = near {
        damage(st, i, 50.0);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wall_blocks_a_ray_through_it() {
        let w = Obb::of(BUILD_PIECES_WALL, Vec3::new(0.0, 1.92, 0.0), 0.0);
        // wall faces z: a ray along z through its middle hits; one beside it misses
        assert!(w.ray(Vec3::new(0.0, 1.0, -3.0), Vec3::new(0.0, 1.0, 3.0)).is_some());
        assert!(w.ray(Vec3::new(4.0, 1.0, -3.0), Vec3::new(4.0, 1.0, 3.0)).is_none());
    }
    #[test]
    fn ramp_can_be_stood_on_part_way_up() {
        let r = Obb::of(BUILD_PIECES_RAMP, Vec3::new(0.0, STOREY * 0.5, 0.0), 0.0);
        // straight down at the middle of the ramp hits near half a storey
        let t = r.ray(Vec3::new(0.0, 5.0, 0.0), Vec3::new(0.0, -1.0, 0.0)).unwrap();
        let y = 5.0 - t;
        assert!((y - STOREY * 0.5).abs() < 0.3, "y {y}");
    }
}
