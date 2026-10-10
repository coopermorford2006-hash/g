//! systems:renderer (first step). Fortnite meshes from the setup tool's cache (.glb + <name>.materials.json),
//! drawn with the mod's own camera through ImGui's foreground draw list: triangles are transformed on the
//! CPU, back faces culled, sorted far to near (no depth buffer) and shaded with one light. No skinning yet:
//! skinned meshes are drawn in their bind pose.
use glam::{Quat, Vec2, Vec3};
use std::collections::HashMap;
use std::path::Path;

pub struct Mesh {
    pub pos: Vec<Vec3>,
    pub nrm: Vec<Vec3>,
    pub uv: Vec<Vec2>,
    /// Triangles as vertex indices, with the material slot of each.
    pub tris: Vec<[u32; 3]>,
    pub tri_mat: Vec<u16>,
    /// glTF material names, indexed by slot.
    pub materials: Vec<String>,
    /// Material name -> texture file (absolute), from <name>.materials.json.
    pub textures: HashMap<String, std::path::PathBuf>,
}

struct Glb {
    json: serde_json::Value,
    bin: Vec<u8>,
}

fn read_glb(path: &Path) -> Option<Glb> {
    let b = std::fs::read(path).ok()?;
    if b.len() < 20 || &b[0..4] != b"glTF" {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes(b[o..o + 4].try_into().unwrap()) as usize;
    let json_len = u32_at(12);
    let json: serde_json::Value = serde_json::from_slice(&b[20..20 + json_len]).ok()?;
    let mut bin = Vec::new();
    let bin_at = 20 + json_len;
    if b.len() >= bin_at + 8 && &b[bin_at + 4..bin_at + 8] == b"BIN\0" {
        let n = u32_at(bin_at);
        bin = b[bin_at + 8..bin_at + 8 + n].to_vec();
    }
    Some(Glb { json, bin })
}

impl Glb {
    /// Reads accessor `i` as f32 tuples of `n` components (floats, or normalized/plain integers).
    fn floats(&self, i: usize, n: usize) -> Option<Vec<f32>> {
        let a = &self.json["accessors"][i];
        let view = &self.json["bufferViews"][a["bufferView"].as_u64()? as usize];
        let count = a["count"].as_u64()? as usize;
        let ctype = a["componentType"].as_u64()?;
        let csize = match ctype { 5126 | 5125 => 4, 5123 | 5122 => 2, 5121 | 5120 => 1, _ => return None };
        let stride = view["byteStride"].as_u64().map(|s| s as usize).unwrap_or(csize * n);
        let base = view["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let norm = a["normalized"].as_bool().unwrap_or(false);
        let mut out = Vec::with_capacity(count * n);
        for e in 0..count {
            for c in 0..n {
                let o = base + e * stride + c * csize;
                let raw = self.bin.get(o..o + csize)?;
                out.push(match ctype {
                    5126 => f32::from_le_bytes(raw.try_into().ok()?),
                    5125 => u32::from_le_bytes(raw.try_into().ok()?) as f32,
                    5123 => { let v = u16::from_le_bytes(raw.try_into().ok()?) as f32; if norm { v / 65535.0 } else { v } }
                    5121 => { let v = raw[0] as f32; if norm { v / 255.0 } else { v } }
                    _ => 0.0,
                });
            }
        }
        Some(out)
    }

    fn indices(&self, i: usize) -> Option<Vec<u32>> {
        let a = &self.json["accessors"][i];
        let view = &self.json["bufferViews"][a["bufferView"].as_u64()? as usize];
        let count = a["count"].as_u64()? as usize;
        let base = view["byteOffset"].as_u64().unwrap_or(0) as usize + a["byteOffset"].as_u64().unwrap_or(0) as usize;
        let size = match a["componentType"].as_u64()? { 5125 => 4, 5123 => 2, 5121 => 1, _ => return None };
        (0..count)
            .map(|e| {
                let o = base + e * size;
                let raw = self.bin.get(o..o + size)?;
                Some(match size { 4 => u32::from_le_bytes(raw.try_into().ok()?), 2 => u16::from_le_bytes(raw.try_into().ok()?) as u32, _ => raw[0] as u32 })
            })
            .collect()
    }
}

/// Loads every triangle primitive of every mesh in a .glb (node transforms ignored: the setup tool's
/// exports keep geometry at the origin, and skinned meshes are in bind pose).
pub fn load(path: &Path) -> Option<Mesh> {
    let g = read_glb(path)?;
    let materials: Vec<String> = g.json["materials"].as_array().map(|a| a.iter().map(|m| m["name"].as_str().unwrap_or("").to_string()).collect()).unwrap_or_default();
    let mut m = Mesh { pos: vec![], nrm: vec![], uv: vec![], tris: vec![], tri_mat: vec![], materials, textures: HashMap::new() };
    for mesh in g.json["meshes"].as_array()? {
        for prim in mesh["primitives"].as_array()? {
            if prim["mode"].as_u64().unwrap_or(4) != 4 {
                continue;
            }
            let at = &prim["attributes"];
            let Some(p) = at["POSITION"].as_u64().and_then(|i| g.floats(i as usize, 3)) else { continue };
            let n = at["NORMAL"].as_u64().and_then(|i| g.floats(i as usize, 3));
            let t = at["TEXCOORD_0"].as_u64().and_then(|i| g.floats(i as usize, 2));
            let base = m.pos.len() as u32;
            let count = p.len() / 3;
            for v in 0..count {
                // glTF is right-handed; Elden Ring's world (camera basis right = up x forward) is left-handed:
                // mirror Z so the model isn't drawn mirror-imaged
                m.pos.push(Vec3::new(p[v * 3], p[v * 3 + 1], -p[v * 3 + 2]));
                m.nrm.push(n.as_ref().map(|n| Vec3::new(n[v * 3], n[v * 3 + 1], -n[v * 3 + 2])).unwrap_or(Vec3::Y));
                m.uv.push(t.as_ref().map(|t| Vec2::new(t[v * 2], t[v * 2 + 1])).unwrap_or(Vec2::ZERO));
            }
            let idx = match prim["indices"].as_u64() { Some(i) => g.indices(i as usize)?, None => (0..count as u32).collect() };
            let mat = prim["material"].as_u64().unwrap_or(0) as u16;
            for t in idx.chunks_exact(3) {
                m.tris.push([base + t[0], base + t[1], base + t[2]]);
                m.tri_mat.push(mat);
            }
        }
    }
    let map_path = path.with_extension("materials.json");
    if let Some(map) = std::fs::read(&map_path).ok().and_then(|b| serde_json::from_slice::<HashMap<String, String>>(&b).ok()) {
        let dir = path.parent().unwrap_or(Path::new("."));
        m.textures = map.into_iter().map(|(k, v)| (k, dir.join(v))).collect();
    }
    Some(m)
}

/// Bounds of a mesh (min, max), for logging what was loaded.
pub fn bounds(m: &Mesh) -> (Vec3, Vec3) {
    m.pos.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)))
}

/// One triangle ready for the draw list.
pub struct ScreenTri {
    pub depth: f32,
    pub tex: u16,
    pub p: [[f32; 2]; 3],
    pub uv: [[f32; 2]; 3],
    pub shade: u8,
}

pub struct View {
    pub pos: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub fwd: Vec3,
    /// focal length for the horizontal FOV, and display size
    pub f: f32,
    pub display: [f32; 2],
}

impl View {
    pub fn new(pos: Vec3, fwd: Vec3, fov_deg: f32, display: [f32; 2]) -> Self {
        let right = Vec3::Y.cross(fwd).normalize_or_zero();
        let up = fwd.cross(right);
        View { pos, right, up, fwd, f: 1.0 / (fov_deg.to_radians() * 0.5).tan(), display }
    }

    fn project(&self, p: Vec3) -> Option<([f32; 2], f32)> {
        let d = p - self.pos;
        let z = d.dot(self.fwd);
        if z < 0.05 {
            return None;
        }
        let x = d.dot(self.right) / z * self.f;
        let y = d.dot(self.up) / z * self.f * (self.display[0] / self.display[1]);
        Some(([self.display[0] * 0.5 * (1.0 + x), self.display[1] * 0.5 * (1.0 - y)], z))
    }
}

/// Places `mesh` at `at` turned by `yaw` and appends its visible triangles (unsorted) to `out`.
/// `tex_slot` maps the mesh's material slots to texture indices in the caller's table.
pub fn emit(mesh: &Mesh, at: Vec3, yaw: f32, view: &View, tex_slot: &[u16], out: &mut Vec<ScreenTri>) {
    let rot = Quat::from_rotation_y(yaw);
    let light = Vec3::new(0.35, 0.85, 0.4).normalize();
    let world: Vec<Vec3> = mesh.pos.iter().map(|p| at + rot * *p).collect();
    let proj: Vec<Option<([f32; 2], f32)>> = world.iter().map(|p| view.project(*p)).collect();
    for (t, tri) in mesh.tris.iter().enumerate() {
        let (Some(a), Some(b), Some(c)) = (proj[tri[0] as usize], proj[tri[1] as usize], proj[tri[2] as usize]) else { continue };
        // no back-face culling until the winding is checked in game: the far-to-near sort keeps it correct
        let area = (b.0[0] - a.0[0]) * (c.0[1] - a.0[1]) - (b.0[1] - a.0[1]) * (c.0[0] - a.0[0]);
        if area.abs() < 0.01 {
            continue;
        }
        let n = rot * (mesh.nrm[tri[0] as usize] + mesh.nrm[tri[1] as usize] + mesh.nrm[tri[2] as usize]).normalize_or_zero();
        // dimmer than full white: Elden Ring's interiors are dark and a fully lit model looks pasted on
        let shade = (0.32 + 0.48 * n.dot(light).max(0.0)).clamp(0.0, 1.0);
        let uv = |i: u32| { let u = mesh.uv[i as usize]; [u.x, u.y] };
        out.push(ScreenTri {
            depth: (a.1 + b.1 + c.1) / 3.0,
            tex: tex_slot.get(mesh.tri_mat[t] as usize).copied().unwrap_or(u16::MAX),
            p: [a.0, b.0, c.0],
            uv: [uv(tri[0]), uv(tri[1]), uv(tri[2])],
            shade: (shade * 255.0) as u8,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Renders the cached outfit the way the game will (same load/emit path) into a PNG, from the front
    /// and from the side, to check orientation and texturing without the game.
    /// cargo test --release preview_outfit -- --ignored --nocapture   (FORTRING_PREVIEW = output folder)
    #[test]
    #[ignore]
    fn preview_outfit() {
        let dir = crate::paths::cache_dir().join("skeletal_mesh");
        let out = std::env::var("FORTRING_PREVIEW").map(std::path::PathBuf::from).unwrap_or_else(|_| std::env::temp_dir());
        let parts: Vec<Mesh> = (0..).map_while(|i| load(&dir.join(if i == 0 { "outfit.glb".to_string() } else { format!("outfit_{i}.glb") }))).collect();
        assert!(!parts.is_empty(), "no outfit in {}", dir.display());
        let mut textures: Vec<(Vec<u8>, u32, u32)> = Vec::new();
        let mut slots: Vec<Vec<u16>> = Vec::new();
        for m in &parts {
            slots.push(m.materials.iter().map(|n| m.textures.get(n).and_then(|p| crate::hud::load_png_file_for_test(p)).map(|t| { textures.push(t); (textures.len() - 1) as u16 }).unwrap_or(u16::MAX)).collect());
        }
        let (w, h) = (480usize, 640usize);
        // camera 3 m in front of the character (character yaw 0, camera looking back at it), and from its right
        for (name, cam_pos) in [("front", Vec3::new(0.0, 1.0, 3.0)), ("side", Vec3::new(3.0, 1.0, 0.0)), ("back", Vec3::new(0.0, 1.0, -3.0))] {
            let fwd = (Vec3::new(0.0, 0.9, 0.0) - cam_pos).normalize();
            let view = View::new(cam_pos, fwd, 40.0, [w as f32, h as f32]);
            let mut tris = Vec::new();
            for (m, s) in parts.iter().zip(&slots) {
                emit(m, Vec3::ZERO, 0.0, &view, s, &mut tris);
            }
            tris.sort_unstable_by(|a, b| b.depth.total_cmp(&a.depth));
            let mut img = vec![40u8; w * h * 4];
            for t in &tris {
                raster(&mut img, w, h, t, textures.get(t.tex as usize));
            }
            let path = out.join(format!("outfit_{name}.png"));
            let f = std::fs::File::create(&path).unwrap();
            let mut e = png::Encoder::new(std::io::BufWriter::new(f), w as u32, h as u32);
            e.set_color(png::ColorType::Rgba);
            e.write_header().unwrap().write_image_data(&img).unwrap();
            println!("{} triangles -> {}", tris.len(), path.display());
        }
    }

    fn raster(img: &mut [u8], w: usize, h: usize, t: &ScreenTri, tex: Option<&(Vec<u8>, u32, u32)>) {
        let [a, b, c] = t.p;
        let area = (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0]);
        if area.abs() < 1e-6 {
            return;
        }
        let (x0, x1) = (a[0].min(b[0]).min(c[0]).max(0.0) as usize, (a[0].max(b[0]).max(c[0]).ceil() as usize).min(w - 1));
        let (y0, y1) = (a[1].min(b[1]).min(c[1]).max(0.0) as usize, (a[1].max(b[1]).max(c[1]).ceil() as usize).min(h - 1));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let w0 = ((b[0] - px) * (c[1] - py) - (b[1] - py) * (c[0] - px)) / area;
                let w1 = ((c[0] - px) * (a[1] - py) - (c[1] - py) * (a[0] - px)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let mut rgb = [200u8, 200, 200];
                if let Some((px_data, tw, th)) = tex {
                    let u = w0 * t.uv[0][0] + w1 * t.uv[1][0] + w2 * t.uv[2][0];
                    let v = w0 * t.uv[0][1] + w1 * t.uv[1][1] + w2 * t.uv[2][1];
                    let tx = ((u.rem_euclid(1.0)) * *tw as f32) as usize % *tw as usize;
                    let ty = ((v.rem_euclid(1.0)) * *th as f32) as usize % *th as usize;
                    let o = (ty * *tw as usize + tx) * 4;
                    rgb = [px_data[o], px_data[o + 1], px_data[o + 2]];
                }
                let o = (y * w + x) * 4;
                for k in 0..3 {
                    img[o + k] = (rgb[k] as u32 * t.shade as u32 / 255) as u8;
                }
                img[o + 3] = 255;
            }
        }
    }
}
