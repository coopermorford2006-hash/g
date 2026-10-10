//! systems:renderer, animation clips. Reads the ActorX .psa files the setup tool exports (CUE4Parse's
//! ActorXAnim writer) and samples bone poses in the glTF space of the exported meshes.
//!
//! Conventions undone here (CUE4Parse-Conversion Writers/ActorX/ActorXAnim.cs and Writers/Gltf/Gltf.cs):
//! - PSA keys are Y-mirrored, non-root rotations stored conjugated (ActorX), the root's W negated;
//! - the glTF exporter maps Unreal (x, y, z) to (x, z, y), quaternions to (x, z, y, -w), and cm to m.
//! Together: non-root q = (x, z, -y, -w), root q = (x, z, -y, w), position = (x, z, -y) / 100.
use glam::{Quat, Vec3};
use std::collections::HashMap;
use std::path::Path;

pub struct Clip {
    pub fps: f32,
    pub frames: usize,
    pub bones: Vec<String>,
    /// frame-major: keys[frame * bones.len() + bone]
    pub keys: Vec<(Vec3, Quat)>,
    pub by_name: HashMap<String, usize>,
}

fn f32_at(b: &[u8], o: usize) -> f32 {
    f32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn i32_at(b: &[u8], o: usize) -> i32 {
    i32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}

fn name(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

pub fn load(path: &Path) -> Option<Clip> {
    let b = std::fs::read(path).ok()?;
    let mut o = 0;
    let (mut bones, mut fps, mut frames, mut raw) = (Vec::new(), 30.0, 0usize, Vec::new());
    while o + 32 <= b.len() {
        let id = name(&b[o..o + 20]);
        let size = i32_at(&b, o + 24).max(0) as usize;
        let count = i32_at(&b, o + 28).max(0) as usize;
        let data = o + 32;
        let end = data + size * count;
        if end > b.len() {
            return None;
        }
        match id.as_str() {
            "BONENAMES" => {
                for i in 0..count {
                    bones.push(name(&b[data + i * size..data + i * size + 64]));
                }
            }
            "ANIMINFO" if count > 0 => {
                fps = f32_at(&b, data + 64 + 64 + 24);
                frames = i32_at(&b, data + 64 + 64 + 36).max(0) as usize;
            }
            "ANIMKEYS" => {
                for i in 0..count {
                    let k = data + i * size;
                    let (px, py, pz) = (f32_at(&b, k), f32_at(&b, k + 4), f32_at(&b, k + 8));
                    let (qx, qy, qz, qw) = (f32_at(&b, k + 12), f32_at(&b, k + 16), f32_at(&b, k + 20), f32_at(&b, k + 24));
                    let bone = if bones.is_empty() { 0 } else { i % bones.len() };
                    let w = if bone == 0 { qw } else { -qw };
                    raw.push((Vec3::new(px, pz, -py) * 0.01, Quat::from_xyzw(qx, qz, -qy, w).normalize()));
                }
            }
            _ => {}
        }
        o = end;
    }
    if bones.is_empty() || raw.is_empty() {
        return None;
    }
    let frames = frames.max(1).min(raw.len() / bones.len());
    let by_name = bones.iter().enumerate().map(|(i, n)| (n.to_ascii_lowercase(), i)).collect();
    Some(Clip { fps: if fps > 0.0 { fps } else { 30.0 }, frames, bones, keys: raw, by_name })
}

impl Clip {
    pub fn duration(&self) -> f32 {
        self.frames as f32 / self.fps
    }

    /// Local (translation, rotation) of `bone` at `t` seconds, looping.
    pub fn sample(&self, bone: usize, t: f32) -> (Vec3, Quat) {
        let n = self.bones.len();
        if self.frames <= 1 {
            return self.keys[bone];
        }
        let f = (t * self.fps).rem_euclid(self.frames as f32);
        let a = f.floor() as usize % self.frames;
        let b = (a + 1) % self.frames;
        let s = f - f.floor();
        let (pa, qa) = self.keys[a * n + bone];
        let (pb, qb) = self.keys[b * n + bone];
        (pa.lerp(pb, s), qa.slerp(qb, s))
    }
}

#[cfg(test)]
mod tests {
    /// Prints rest poses three ways for a few bones: glTF node, PSA BONENAMES (raw Unreal) and frame 0 key.
    #[test]
    #[ignore]
    fn compare_rest() {
        let cache = crate::paths::cache_dir();
        let mesh = crate::model::load(&cache.join("skeletal_mesh/outfit_1.glb")).unwrap();
        let path = cache.join("anim/anim_jog.psa");
        let b = std::fs::read(&path).unwrap();
        let clip = super::load(&path).unwrap();
        // BONENAMES raw
        let mut o = 0;
        let mut raw = std::collections::HashMap::new();
        while o + 32 <= b.len() {
            let id = super::name(&b[o..o + 20]);
            let size = super::i32_at(&b, o + 24) as usize;
            let count = super::i32_at(&b, o + 28) as usize;
            if id == "BONENAMES" {
                for i in 0..count {
                    let e = o + 32 + i * size;
                    let n = super::name(&b[e..e + 64]).to_ascii_lowercase();
                    let q: Vec<f32> = (0..4).map(|k| super::f32_at(&b, e + 76 + k * 4)).collect();
                    let p: Vec<f32> = (0..3).map(|k| super::f32_at(&b, e + 92 + k * 4)).collect();
                    raw.insert(n, (q, p));
                }
            }
            o += 32 + size * count;
        }
        for name in ["root", "pelvis", "spine_01", "thigh_l", "calf_l", "upperarm_l", "head"] {
            let node = mesh.nodes.iter().find(|n| n.name == name);
            let key = clip.by_name.get(name).map(|&i| clip.keys[i]);
            println!("{name}\n  gltf node   t {:?} r {:?}\n  psa rest    {:?}\n  frame0 key  {:?}", node.map(|n| n.t), node.map(|n| n.r), raw.get(name), key);
        }
    }
}

#[cfg(test)]
mod tests_frames {
    #[test]
    #[ignore]
    fn frames() {
        for clip_name in ["anim_jog", "anim_idle", "anim_sprint", "anim_pickaxe_swing"] {
            let c = super::load(&crate::paths::cache_dir().join(format!("anim/{clip_name}.psa"))).unwrap();
            let n = c.bones.len();
            let moving = (0..n).filter(|&b| (0..c.frames).any(|f| c.keys[f * n + b].1.angle_between(c.keys[b].1) > 0.01)).count();
            let nonzero_t = (0..n).filter(|&b| c.keys[b].0.length() > 1e-4).count();
            println!("{clip_name}: {} frames, {n} bones, {moving} bones rotate over time, {nonzero_t} with non-zero translation at frame 0", c.frames);
            for name in ["pelvis", "thigh_l", "calf_l"] {
                if let Some(&b) = c.by_name.get(name) {
                    let qs: Vec<String> = (0..c.frames.min(4)).map(|f| format!("{:.3?}", c.keys[f * n + b].1.to_array())).collect();
                    println!("  {name}: {}", qs.join(" "));
                }
            }
        }
    }
}
