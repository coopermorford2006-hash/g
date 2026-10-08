//! systems:health_shield + consumables_use + death. Fortnite's 100 HP + 100 shield on top of Elden
//! Ring's HP: health shown = 100 x HP / max HP, so Vigor levels still matter. Shield is the mod's own
//! pool (in Fortnite points) and soaks damage first: when Elden Ring's HP drops and shield is left,
//! the HP is restored and the shield pays instead.
use crate::generated::*;
use crate::input::Frame;
use crate::state::State;
use crate::world;

pub fn fortnite_hp(hp: i32, max: i32) -> f32 {
    if max <= 0 { 0.0 } else { (100.0 * hp as f32 / max as f32).clamp(0.0, 100.0) }
}

pub fn to_er(points: f32, max: i32) -> i32 {
    (points / 100.0 * max as f32).round() as i32
}

/// Splits a hit between shield and health. Returns (new shield, ER HP to give back).
pub fn absorb(shield: f32, drop_er: i32, max: i32) -> (f32, i32) {
    let drop_pts = 100.0 * drop_er as f32 / max.max(1) as f32;
    let soaked = drop_pts.min(shield);
    (shield - soaked, to_er(soaked, max).min(drop_er))
}

pub fn tick(st: &mut State, inp: &Frame, dt: f32, fall_damage_pts: f32) {
    let Some(p) = world::player() else { return };
    let data = &mut p.chr_ins.modules.data;
    let (hp, max) = (data.hp, data.max_hp);
    if max <= 0 {
        return;
    }
    if st.last_hp >= 0 && hp < st.last_hp && hp > 0 && st.saved.shield > 0.0 {
        let (shield, back) = absorb(st.saved.shield, st.last_hp - hp, max);
        st.saved.shield = shield;
        data.hp = (hp + back).min(max);
        st.dirty = true;
    }
    if fall_damage_pts > 0.0 {
        data.hp = (data.hp - to_er(fall_damage_pts, max)).max(0);
    }
    if data.hp <= 0 && st.last_hp > 0 {
        st.eliminated = 4.0;
        st.using = None;
        crate::log!("health: eliminated");
    }
    st.eliminated = (st.eliminated - dt).max(0.0);

    // using a consumable (Fortnite: you can walk while using, firing or swapping cancels)
    if let Some(c) = st.using {
        let row = &CONSUMABLES[c];
        if inp.pressed(CONTROLS_FIRE) || inp.pressed(CONTROLS_RELOAD) || data.hp <= 0 {
            st.using = None;
            st.progress = None;
        } else {
            st.use_left -= dt;
            st.progress = Some((row.name.to_string(), 1.0 - st.use_left / row.use_s));
            if st.use_left <= 0.0 {
                apply(st, c, &mut data.hp, max);
                st.using = None;
                st.progress = None;
            }
        }
    } else if inp.pressed(CONTROLS_HEAL_ITEM) {
        let c = st.selected_consumable;
        if st.saved.consumables[c] > 0 && can_use(st, c, data.hp, max) {
            st.using = Some(c);
            st.use_left = CONSUMABLES[c].use_s;
            crate::audio::play(CONSUMABLES[c].fn_use_sound, None);
        } else if st.saved.consumables[c] == 0 {
            // cycle to the next consumable the player has
            if let Some(n) = (1..=CONSUMABLES.len()).map(|i| (c + i) % CONSUMABLES.len()).find(|&i| st.saved.consumables[i] > 0) {
                st.selected_consumable = n;
            }
        }
    }
    st.last_hp = data.hp;
}

fn can_use(st: &State, c: usize, hp: i32, max: i32) -> bool {
    let row = &CONSUMABLES[c];
    match row.restores {
        "health" => fortnite_hp(hp, max) < row.cap as f32,
        _ => st.saved.shield < row.cap as f32,
    }
}

fn apply(st: &mut State, c: usize, hp: &mut i32, max: i32) {
    let row = &CONSUMABLES[c];
    st.saved.consumables[c] -= 1;
    st.dirty = true;
    match row.restores {
        "health" => {
            let target = (fortnite_hp(*hp, max) + row.amount as f32).min(row.cap as f32);
            *hp = (*hp).max(to_er(target, max)).min(max);
        }
        _ => st.saved.shield = (st.saved.shield + row.amount as f32).min(row.cap as f32),
    }
    crate::log!("health: used {} (hp {} shield {:.0})", row.name, *hp, st.saved.shield);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shield_soaks_first() {
        // 1000 max HP: a 300 HP hit is 30 points; 50 shield soaks all of it
        assert_eq!(absorb(50.0, 300, 1000), (20.0, 300));
        // 10 shield soaks 10 points = 100 HP of a 300 HP hit
        let (s, back) = absorb(10.0, 300, 1000);
        assert_eq!((s, back), (0.0, 100));
    }
    #[test]
    fn hp_scale() {
        assert_eq!(fortnite_hp(500, 1000), 50.0);
        assert_eq!(to_er(75.0, 1200), 900);
    }
}
