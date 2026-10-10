//! The mod's own game state: inventory, mode, health/shield, chests and builds. It is saved in a
//! sidecar file next to the mod's Elden Ring save (systems:inventory).
use crate::generated::*;
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Gun {
    pub weapon: usize,
    pub rarity: usize,
    pub tier: usize,
    pub mag: u32,
}

impl Gun {
    pub fn row(&self) -> &'static WeaponsRow {
        &WEAPONS[self.weapon]
    }
    /// Elden Ring damage per shot: Fortnite damage x the tier it was found in.
    pub fn damage(&self) -> f32 {
        self.row().damage[RARITIES[self.rarity].index as usize] * REGION_TIERS[self.tier].damage_mult
    }
    pub fn reload_s(&self) -> f32 {
        self.row().reload_s[RARITIES[self.rarity].index as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Held {
    Pickaxe,
    Slot(usize),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Combat,
    Build,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Build {
    pub piece: usize,
    pub material: usize,
    /// Havok-space position of the piece's centre and its yaw in radians.
    pub pos: [f32; 3],
    pub yaw: f32,
    pub hp: f32,
    pub max_hp: f32,
    pub placed_at: f64,
    /// Elden Ring block (map tile) id it was placed in, so it reloads in the right place.
    pub block: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pickup {
    pub item: crate::generated::Ref,
    pub gun: Option<Gun>,
    pub count: u32,
    pub pos: [f32; 3],
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Saved {
    pub slots: [Option<Gun>; 5],
    pub ammo: [u32; 5],
    pub materials: [u32; 3],
    pub consumables: [u32; 4],
    pub shield: f32,
    /// Containers already searched, keyed "map:entry".
    pub opened: Vec<String>,
    pub builds: Vec<Build>,
}

pub struct State {
    pub saved: Saved,
    pub held: Held,
    pub mode: Mode,
    pub build_piece: usize,
    pub build_material: usize,
    pub build_rotation: u32,
    pub selected_consumable: usize,
    /// Camera yaw/pitch (radians), owned by the mod while it drives.
    pub yaw: f32,
    pub pitch: f32,
    pub ads: f32,
    pub crouched: bool,
    pub bloom: f32,
    pub fire_cooldown: f32,
    pub reload_left: f32,
    pub use_left: f32,
    pub using: Option<usize>,
    pub last_hp: i32,
    pub hit_marker: f32,
    pub pickups: Vec<Pickup>,
    pub prompt: Option<String>,
    pub prompt_color: [f32; 4],
    pub progress: Option<(String, f32)>,
    pub damage_numbers: Vec<([f32; 3], f32, f32, u8)>,
    /// Shot tracers drawn by the HUD: (from, to, seconds left).
    pub tracers: Vec<([f32; 3], [f32; 3], f32)>,
    pub eliminated: f32,
    pub message: Option<(String, f32)>,
    pub slot: u32,
    pub dirty: bool,
}

impl State {
    pub const fn new() -> Self {
        State {
            saved: Saved {
                slots: [None; 5],
                ammo: [0; 5],
                materials: [0; 3],
                consumables: [0; 4],
                shield: 0.0,
                opened: Vec::new(),
                builds: Vec::new(),
            },
            held: Held::Pickaxe,
            mode: Mode::Combat,
            build_piece: BUILD_PIECES_WALL,
            build_material: MATERIALS_WOOD,
            build_rotation: 0,
            selected_consumable: 0,
            yaw: 0.0,
            pitch: 0.0,
            ads: 0.0,
            crouched: false,
            bloom: 0.0,
            fire_cooldown: 0.0,
            reload_left: 0.0,
            use_left: 0.0,
            using: None,
            last_hp: -1,
            hit_marker: 0.0,
            pickups: Vec::new(),
            prompt: None,
            prompt_color: [1.0; 4],
            progress: None,
            damage_numbers: Vec::new(),
            tracers: Vec::new(),
            eliminated: 0.0,
            message: None,
            slot: 0,
            dirty: false,
        }
    }

    pub fn held_gun(&self) -> Option<Gun> {
        match self.held {
            Held::Slot(i) => self.saved.slots[i],
            Held::Pickaxe => None,
        }
    }

    pub fn held_gun_mut(&mut self) -> Option<&mut Gun> {
        match self.held {
            Held::Slot(i) => self.saved.slots[i].as_mut(),
            Held::Pickaxe => None,
        }
    }

    pub fn load(&mut self, slot: u32) {
        self.slot = slot;
        let path = crate::paths::sidecar(slot);
        self.saved = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        // containers that refill (ammo boxes, spawn_rules respawn "on_rest") are closed again on load
        let refills: Vec<&str> = crate::generated::CONTAINERS.iter()
            .filter(|c| crate::generated::SPAWN_RULES[c.placement].respawn != "never")
            .map(|c| c.id)
            .collect();
        self.saved.opened.retain(|k| !refills.iter().any(|id| k.contains(&format!(":{id}:"))));
        crate::log!("state: loaded slot {slot} ({} builds, {} chests opened)", self.saved.builds.len(), self.saved.opened.len());
    }

    pub fn save(&mut self) {
        let path = crate::paths::sidecar(self.slot);
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(b) = serde_json::to_vec(&self.saved) {
            let tmp = path.with_extension("tmp");
            if std::fs::write(&tmp, b).is_ok() {
                let _ = std::fs::rename(&tmp, &path);
            }
        }
        self.dirty = false;
    }

    pub fn message(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), 3.0));
    }
}

pub static STATE: Mutex<State> = Mutex::new(State::new());

#[cfg(test)]
impl State {
    pub fn new_for_tests() -> Self {
        Self::new()
    }
}
