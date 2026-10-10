//! systems:hud. Fortnite's HUD drawn with ImGui over Elden Ring's frame (hudhook DX12). Layout comes
//! from sheets/hud.json; icons, rarity backgrounds and the Burbank font come from the setup tool's
//! cache (the player's own Fortnite install). Missing art falls back to plain shapes in Fortnite colours.
use crate::generated::*;
use crate::state::{Held, Mode, STATE};
use glam::Vec3;
use hudhook::imgui::{self, Context, FontSource, ImColor32, TextureId, Ui};
use hudhook::{ImguiRenderLoop, RenderContext};
use std::collections::HashMap;

/// Added to the player's yaw when drawing the outfit: the exported models face -Z (checked with
/// model::tests::preview_outfit), the movement code faces +Z at yaw 0.
const MODEL_YAW_OFFSET: f32 = std::f32::consts::PI;

pub struct Hud {
    tex: HashMap<usize, (TextureId, [f32; 2])>,
    big_font: Option<imgui::FontId>,
    /// The Fortnite outfit (every part), each with its material slot -> model_tex index table.
    outfit: Vec<(crate::model::Mesh, Vec<u16>)>,
    model_tex: Vec<TextureId>,
    tris: Vec<crate::model::ScreenTri>,
}

// The HUD lives on the render thread only; hudhook requires Send + Sync for the render loop.
unsafe impl Send for Hud {}
unsafe impl Sync for Hud {}

impl Hud {
    /// outfit.glb, outfit_1.glb, ... (the outfit's parts) and their material textures.
    fn load_outfit(&mut self, rc: &mut dyn RenderContext) {
        let first = crate::paths::cache_dir().join(FORTNITE_ASSETS[FORTNITE_ASSETS_OUTFIT].out);
        let mut loaded: HashMap<std::path::PathBuf, u16> = HashMap::new();
        for i in 0.. {
            let path = if i == 0 { first.clone() } else { first.with_file_name(format!("{}_{i}.glb", first.file_stem().unwrap().to_string_lossy())) };
            let Some(mesh) = crate::model::load(&path) else { break };
            let mut slots = Vec::new();
            for name in &mesh.materials {
                let slot = mesh.textures.get(name).and_then(|png| {
                    if let Some(&s) = loaded.get(png) {
                        return Some(s);
                    }
                    let (rgba, w, h) = load_png_file(png)?;
                    let id = rc.load_texture(&rgba, w, h).ok()?;
                    self.model_tex.push(id);
                    let s = (self.model_tex.len() - 1) as u16;
                    loaded.insert(png.clone(), s);
                    Some(s)
                });
                slots.push(slot.unwrap_or(u16::MAX));
            }
            let (lo, hi) = crate::model::bounds(&mesh);
            crate::log!("model: {} - {} vertices, {} triangles, materials {:?} -> textures {:?}, bounds {lo:.2}..{hi:.2}",
                path.file_name().unwrap().to_string_lossy(), mesh.pos.len(), mesh.tris.len(), mesh.materials, slots);
            self.outfit.push((mesh, slots));
        }
        if self.outfit.is_empty() {
            crate::log!("model: no outfit at {}", first.display());
        }
    }

    /// The Fortnite character at the player's feet (drawn under the HUD).
    fn draw_outfit(&mut self, display: [f32; 2]) {
        let Some((cam, body)) = crate::tick::FRAME.lock().ok().and_then(|f| Some((f.cam?, f.body?))) else { return };
        if self.outfit.is_empty() {
            return;
        }
        let view = crate::model::View::new(cam.0, cam.1, cam.2, display);
        self.tris.clear();
        for (mesh, slots) in &self.outfit {
            crate::model::emit(mesh, body.0, body.1 + MODEL_YAW_OFFSET, &view, slots, &mut self.tris);
        }
        self.tris.sort_unstable_by(|a, b| b.depth.total_cmp(&a.depth));
        use hudhook::imgui::sys;
        let v2 = |p: [f32; 2]| sys::ImVec2 { x: p[0], y: p[1] };
        unsafe {
            let dl = sys::igGetForegroundDrawList();
            let mut current: Option<u16> = None;
            for t in &self.tris {
                let g = t.shade as u32;
                if t.tex == u16::MAX {
                    if current.take().is_some() {
                        sys::ImDrawList_PopTextureID(dl);
                    }
                    let c = 0xFF00_0000 | (g * 200 / 255) << 16 | (g * 200 / 255) << 8 | (g * 200 / 255);
                    sys::ImDrawList_AddTriangleFilled(dl, v2(t.p[0]), v2(t.p[1]), v2(t.p[2]), c);
                    continue;
                }
                if current != Some(t.tex) {
                    if current.is_some() {
                        sys::ImDrawList_PopTextureID(dl);
                    }
                    sys::ImDrawList_PushTextureID(dl, self.model_tex[t.tex as usize].id() as sys::ImTextureID);
                    current = Some(t.tex);
                }
                let c = 0xFF00_0000 | g << 16 | g << 8 | g;
                sys::ImDrawList_PrimReserve(dl, 3, 3);
                for k in 0..3 {
                    sys::ImDrawList_PrimVtx(dl, v2(t.p[k]), v2(t.uv[k]), c);
                }
            }
            if current.is_some() {
                sys::ImDrawList_PopTextureID(dl);
            }
        }
    }

    pub fn new() -> Self {
        Hud { tex: HashMap::new(), big_font: None, outfit: Vec::new(), model_tex: Vec::new(), tris: Vec::new() }
    }
}

fn load_png(asset: usize) -> Option<(Vec<u8>, u32, u32)> {
    load_png_file(&crate::paths::cache_dir().join(FORTNITE_ASSETS[asset].out))
}

fn load_png_file(path: &std::path::Path) -> Option<(Vec<u8>, u32, u32)> {
    let file = std::fs::File::open(path).ok()?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut r = dec.read_info().ok()?;
    let mut buf = vec![0; r.output_buffer_size()];
    let info = r.next_frame(&mut buf).ok()?;
    let px = (info.width * info.height) as usize;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..px * 4].to_vec(),
        png::ColorType::Rgb => buf[..px * 3].chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf[..px * 2].chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf[..px].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        _ => return None,
    };
    Some((rgba, info.width, info.height))
}

fn col(c: [f32; 4]) -> ImColor32 {
    ImColor32::from_rgba_f32s(c[0], c[1], c[2], c[3])
}

/// Projects a Havok-space point to screen pixels with the mod's camera.
pub fn project(p: Vec3, display: [f32; 2]) -> Option<[f32; 2]> {
    let (pos, fwd, fov) = crate::tick::FRAME.lock().ok()?.cam?;
    let right = Vec3::Y.cross(fwd).normalize_or_zero();
    let up = fwd.cross(right);
    let d = p - pos;
    let z = d.dot(fwd);
    if z < 0.1 {
        return None;
    }
    let f = 1.0 / (fov.to_radians() * 0.5).tan(); // horizontal FOV
    let x = d.dot(right) / z * f;
    let y = d.dot(up) / z * f * (display[0] / display[1]);
    Some([display[0] * 0.5 * (1.0 + x), display[1] * 0.5 * (1.0 - y)])
}

impl ImguiRenderLoop for Hud {
    fn initialize<'a>(&'a mut self, ctx: &mut Context, rc: &'a mut dyn RenderContext) {
        for (i, a) in FORTNITE_ASSETS.iter().enumerate() {
            if a.kind != "texture" {
                continue;
            }
            if let Some((rgba, w, h)) = load_png(i) {
                if let Ok(id) = rc.load_texture(&rgba, w, h) {
                    self.tex.insert(i, (id, [w as f32, h as f32]));
                }
            }
        }
        crate::log!("hud: {} Fortnite textures loaded", self.tex.len());
        self.load_outfit(rc);
        let font = crate::paths::cache_dir().join(FORTNITE_ASSETS[FORTNITE_ASSETS_FONT_BURBANK].out);
        // ImGui asserts (and the game aborts) on bytes stb_truetype can't parse: only hand it a real sfnt
        let is_sfnt = |b: &[u8]| matches!(b.get(..4), Some([0, 1, 0, 0]) | Some(b"OTTO") | Some(b"true") | Some(b"ttcf"));
        if let Some(ttf) = std::fs::read(&font).ok().filter(|b| is_sfnt(b)) {
            let ttf: &'static [u8] = Box::leak(ttf.into_boxed_slice());
            ctx.fonts().add_font(&[FontSource::TtfData { data: ttf, size_pixels: 22.0, config: None }]);
            self.big_font = Some(ctx.fonts().add_font(&[FontSource::TtfData { data: ttf, size_pixels: 44.0, config: None }]));
            crate::log!("hud: Burbank font loaded");
        } else {
            crate::log!("hud: Burbank font missing or not a TrueType/OpenType file, using the default font");
        }
    }

    fn render(&mut self, ui: &mut Ui) {
        let in_game = crate::tick::FRAME.lock().map(|f| f.cam.is_some()).unwrap_or(false);
        let display = ui.io().display_size;
        if in_game {
            self.draw_outfit(display);
        }
        let dl = ui.get_foreground_draw_list();
        let Ok(st) = STATE.lock() else { return };
        match crate::assets::missing() {
            None => dl.add_text([24.0, 24.0], col([1.0, 0.85, 0.3, 1.0]), "Fortnite Ring: Fortnite content not found. Press Play in Melty again to run the Fortnite setup."),
            Some(0) => {}
            Some(n) => dl.add_text(
                [24.0, 24.0],
                col([1.0, 0.85, 0.3, 0.8]),
                format!("Fortnite Ring: {n} Fortnite assets could not be converted (see %LOCALAPPDATA%\\FortniteRing\\setup.log)"),
            ),
        }
        if !in_game {
            return;
        }
        let px = |r: usize| -> ([f32; 2], [f32; 2]) {
            let row = &HUD[r];
            ([row.pos[0] * display[0], row.pos[1] * display[1]], [row.size[0] * display[0], row.size[1] * display[1]])
        };
        let white = col([1.0, 1.0, 1.0, 1.0]);
        let shadow = col([0.0, 0.0, 0.0, 0.55]);

        // health and shield bars (Fortnite: green health, blue shield, number at the left)
        let (hp, max) = crate::world::player().map(|p| (p.chr_ins.modules.data.hp, p.chr_ins.modules.data.max_hp)).unwrap_or((0, 1));
        let health = crate::health::fortnite_hp(hp, max);
        for (r, value, fill) in [(HUD_HEALTH_BAR, health, [0.36, 0.86, 0.29, 1.0]), (HUD_SHIELD_BAR, st.saved.shield, [0.25, 0.62, 1.0, 1.0])] {
            let (p, s) = px(r);
            dl.add_rect([p[0] + 40.0, p[1]], [p[0] + 40.0 + s[0], p[1] + s[1]], shadow).filled(true).build();
            dl.add_rect([p[0] + 40.0, p[1]], [p[0] + 40.0 + s[0] * value / 100.0, p[1] + s[1]], col(fill)).filled(true).build();
            dl.add_text([p[0], p[1] - 4.0], white, format!("{:.0}", value.ceil()));
        }

        // materials (right side, Fortnite's stacked wood/stone/metal)
        let (p, s) = px(HUD_MATERIALS);
        for (i, m) in MATERIALS.iter().enumerate() {
            let y = p[1] + i as f32 * s[1] / 3.0;
            let selected = st.mode == Mode::Build && st.build_material == i;
            if let Some((t, _)) = self.tex.get(&m.fn_icon) {
                dl.add_image(*t, [p[0], y], [p[0] + 32.0, y + 32.0]).build();
            }
            dl.add_text([p[0] + 40.0, y + 6.0], if selected { col([1.0, 0.85, 0.3, 1.0]) } else { white }, format!("{}", st.saved.materials[i]));
        }

        // runes, as Fortnite's gold counter
        let (p, _) = px(HUD_RUNES);
        let runes = crate::world::player().map(|pl| unsafe { pl.player_game_data.as_ref() }.rune_count).unwrap_or(0);
        if let Some((t, _)) = self.tex.get(&HUD[HUD_RUNES].art.unwrap_or(usize::MAX)) {
            dl.add_image(*t, [p[0] - 30.0, p[1]], [p[0] - 4.0, p[1] + 26.0]).build();
        }
        dl.add_text(p, col([1.0, 0.84, 0.36, 1.0]), format!("{runes}"));

        if st.mode == Mode::Build {
            let (p, s) = px(HUD_BUILD_BAR);
            let w = s[0] / BUILD_PIECES.len() as f32;
            for (i, b) in BUILD_PIECES.iter().enumerate() {
                let x = p[0] + i as f32 * w;
                let sel = st.build_piece == i;
                dl.add_rect([x + 2.0, p[1]], [x + w - 2.0, p[1] + s[1]], col(if sel { [0.15, 0.55, 0.95, 0.85] } else { [0.05, 0.1, 0.2, 0.6] })).filled(true).build();
                dl.add_text([x + 8.0, p[1] + 6.0], white, b.name);
                dl.add_text([x + 8.0, p[1] + s[1] - 26.0], white, format!("{} {}", MATERIALS[st.build_material].piece_cost, MATERIALS[st.build_material].name));
            }
        } else {
            // hotbar: pickaxe + 5 slots with rarity backgrounds
            let (p, s) = px(HUD_HOTBAR);
            let w = s[0] / 6.0;
            for i in 0..6 {
                let x = p[0] + i as f32 * w;
                let (gun, selected) = if i == 0 { (None, st.held == Held::Pickaxe) } else { (st.saved.slots[i - 1], st.held == Held::Slot(i - 1)) };
                let lift = if selected { -10.0 } else { 0.0 };
                let (a, b) = ([x + 3.0, p[1] + lift], [x + w - 3.0, p[1] + s[1] + lift]);
                match gun {
                    Some(g) => {
                        match self.tex.get(&RARITIES[g.rarity].fn_slot_bg) {
                            Some((t, _)) => dl.add_image(*t, a, b).build(),
                            None => dl.add_rect(a, b, col(crate::loot::hex_color(RARITIES[g.rarity].color))).filled(true).rounding(4.0).build(),
                        }
                        if let Some((t, _)) = self.tex.get(&g.row().fn_icon) {
                            dl.add_image(*t, [a[0] + 4.0, a[1] + 4.0], [b[0] - 4.0, b[1] - 4.0]).build();
                        } else {
                            dl.add_text([a[0] + 6.0, a[1] + 6.0], white, g.row().name);
                        }
                        dl.add_text([a[0] + 6.0, b[1] - 24.0], white, format!("{}", g.mag));
                    }
                    None => {
                        dl.add_rect(a, b, col([0.05, 0.08, 0.15, 0.55])).filled(true).rounding(4.0).build();
                        if i == 0 {
                            dl.add_text([a[0] + 6.0, a[1] + 6.0], white, "Pickaxe");
                        }
                    }
                }
                if selected {
                    dl.add_rect(a, b, white).thickness(3.0).rounding(4.0).build();
                }
            }
            // ammo: mag / reserve
            if let Some(g) = st.held_gun() {
                let (p, _) = px(HUD_AMMO_COUNTER);
                dl.add_text(p, white, format!("{}  /  {}", g.mag, st.saved.ammo[g.row().ammo]));
            }
            // consumable slot
            let (p, s) = px(HUD_CONSUMABLE_SLOT);
            let c = st.selected_consumable;
            dl.add_rect(p, [p[0] + s[0], p[1] + s[1]], col([0.05, 0.08, 0.15, 0.55])).filled(true).rounding(4.0).build();
            if let Some((t, _)) = self.tex.get(&CONSUMABLES[c].fn_icon) {
                dl.add_image(*t, [p[0] + 4.0, p[1] + 4.0], [p[0] + s[0] - 4.0, p[1] + s[1] - 4.0]).build();
            }
            dl.add_text([p[0] + 6.0, p[1] + s[1] - 24.0], white, format!("{} [C]", st.saved.consumables[c]));
        }

        // crosshair with live bloom (Fortnite's four-tick reticle), hidden for the pickaxe
        let (p, _) = px(HUD_CROSSHAIR);
        if st.mode == Mode::Combat {
            if let Some(g) = st.held_gun() {
                let row = g.row();
                let spread = row.spread_hip_deg + (row.spread_ads_deg - row.spread_hip_deg) * st.ads + st.bloom;
                let gap = 4.0 + spread * 6.0;
                for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                    let a = [p[0] + dx * gap, p[1] + dy * gap];
                    let b = [p[0] + dx * (gap + 9.0), p[1] + dy * (gap + 9.0)];
                    dl.add_line(a, b, shadow).thickness(4.0).build();
                    dl.add_line(a, b, white).thickness(2.0).build();
                }
            } else {
                dl.add_circle(p, 3.0, white).filled(true).build();
            }
        }
        if st.hit_marker > 0.0 {
            for (dx, dy) in [(1.0, 1.0), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
                dl.add_line([p[0] + dx * 6.0, p[1] + dy * 6.0], [p[0] + dx * 14.0, p[1] + dy * 14.0], white).thickness(2.5).build();
            }
        }

        // floating damage numbers (white body, yellow head; harvest: white / blue crit)
        for (pos, amount, life, kind) in st.damage_numbers.iter() {
            if let Some(s) = project(Vec3::from(*pos) + Vec3::Y * (1.0 - life) * 0.6, display) {
                let c = match kind { 1 => [1.0, 0.86, 0.2, *life], 2 => [0.4, 0.75, 1.0, *life], _ => [1.0, 1.0, 1.0, *life] };
                dl.add_text(s, col(c), format!("{:.0}", amount));
            }
        }

        // chests and pickups in view: glow marker + name (the 3D models are drawn by renderer.rs)
        for c in crate::loot::near_containers() {
            if let Some(s) = project(c.pos + Vec3::Y * 0.6, display) {
                let glow = crate::loot::hex_color(CONTAINERS[c.kind].glow);
                dl.add_circle(s, 10.0, col([glow[0], glow[1], glow[2], 0.35])).filled(true).build();
            }
        }
        for pk in st.pickups.iter() {
            if let Some(s) = project(Vec3::from(pk.pos), display) {
                let (name, c) = crate::loot::describe(pk);
                dl.add_text([s[0] - 40.0, s[1] - 18.0], col(c), name);
            }
        }

        if let Some(text) = &st.prompt {
            let (p, _) = px(HUD_PICKUP_PROMPT);
            dl.add_text([p[0] - 80.0, p[1]], col(st.prompt_color), text);
        }
        if let Some((label, frac)) = &st.progress {
            let (p, s) = px(HUD_USE_PROGRESS);
            dl.add_rect([p[0] - s[0] * 0.5, p[1]], [p[0] + s[0] * 0.5, p[1] + s[1]], shadow).filled(true).build();
            dl.add_rect([p[0] - s[0] * 0.5, p[1]], [p[0] - s[0] * 0.5 + s[0] * frac, p[1] + s[1]], white).filled(true).build();
            dl.add_text([p[0] - s[0] * 0.5, p[1] - 24.0], white, label);
        }
        if let Some((m, _)) = &st.message {
            dl.add_text([display[0] * 0.5 - 80.0, display[1] * 0.3], col([1.0, 0.4, 0.3, 1.0]), m);
        }
        if st.eliminated > 0.0 {
            let _tok = self.big_font.map(|f| ui.push_font(f));
            dl.add_text([display[0] * 0.5 - 120.0, display[1] * 0.4], col([1.0, 0.2, 0.25, (st.eliminated / 2.0).min(1.0)]), "ELIMINATED");
        }
        // boss bar: Elden Ring's own boss tracking, restyled
        if let Some(fe) = crate::world::fe_man() {
            let (p, s) = px(HUD_BOSS_BAR);
            for (i, b) in fe.boss_health_displays.iter().enumerate() {
                if b.fmg_id <= 0 {
                    continue;
                }
                let Some((hp, max)) = crate::world::hp_of(&b.field_ins_handle) else { continue };
                let y = p[1] + i as f32 * (s[1] + 28.0);
                let frac = hp as f32 / max.max(1) as f32;
                dl.add_rect([p[0], y], [p[0] + s[0], y + s[1]], shadow).filled(true).build();
                dl.add_rect([p[0], y], [p[0] + s[0] * frac, y + s[1]], col([0.85, 0.15, 0.2, 1.0])).filled(true).build();
                dl.add_text([p[0], y - 22.0], white, format!("BOSS  {hp} / {max}"));
            }
        }
    }
}

#[cfg(test)]
pub fn load_png_file_for_test(path: &std::path::Path) -> Option<(Vec<u8>, u32, u32)> {
    load_png_file(path)
}
