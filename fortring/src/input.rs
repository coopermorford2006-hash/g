//! systems:input. Elden Ring reads keyboard and mouse only through DirectInput GetDeviceState
//! (CS2 conversion field note). The hook records the real state for the mod, hides the keys the mod owns
//! from the game while Fortnite controls are active, and presses Elden Ring's own key for passthrough
//! actions (controls.er_key).
use crate::generated::{CONTROLS, ControlsRow};
use std::ffi::c_void;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};
use windows::Win32::Devices::HumanInterfaceDevice::{
    DirectInput8Create, GUID_SysKeyboard, GUID_SysMouse, IDirectInput8W, IDirectInputDevice8W,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::core::Interface;

const DIRECTINPUT_VERSION: u32 = 0x0800;
const MOUSE_BUTTONS: usize = 8;

#[derive(Clone)]
pub struct Frame {
    pub keys: [bool; 256],
    pub prev_keys: [bool; 256],
    pub mouse: [bool; MOUSE_BUTTONS],
    pub prev_mouse: [bool; MOUSE_BUTTONS],
    pub dx: f32,
    pub dy: f32,
    pub wheel: f32,
}

struct Raw {
    keys: [bool; 256],
    mouse: [bool; MOUSE_BUTTONS],
    dx: f32,
    dy: f32,
    wheel: f32,
}

impl Default for Frame {
    fn default() -> Self {
        Frame { keys: [false; 256], prev_keys: [false; 256], mouse: [false; MOUSE_BUTTONS], prev_mouse: [false; MOUSE_BUTTONS], dx: 0.0, dy: 0.0, wheel: 0.0 }
    }
}

impl Default for Raw {
    fn default() -> Self {
        Raw { keys: [false; 256], mouse: [false; MOUSE_BUTTONS], dx: 0.0, dy: 0.0, wheel: 0.0 }
    }
}

static RAW: Mutex<Option<Raw>> = Mutex::new(None);
static LAST: Mutex<Option<Frame>> = Mutex::new(None);
/// True while Fortnite controls drive the player (false in Elden Ring menus, dialogue, riding, cutscenes).
pub static MOD_OWNS_INPUT: AtomicBool = AtomicBool::new(false);
/// Elden Ring keys (DIK codes) the mod wants pressed this frame for passthrough actions.
static PASSTHROUGH: Mutex<[bool; 256]> = Mutex::new([false; 256]);
static ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

type GetDeviceState = unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32;

#[repr(C)]
struct DiMouseState2 {
    x: i32,
    y: i32,
    z: i32,
    buttons: [u8; 8],
}

unsafe extern "system" fn detour(this: *mut c_void, size: u32, data: *mut c_void) -> i32 {
    let orig: GetDeviceState = unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    let hr = unsafe { orig(this, size, data) };
    if hr < 0 || data.is_null() {
        return hr;
    }
    let owns = MOD_OWNS_INPUT.load(Ordering::Relaxed);
    let mut raw = RAW.lock().unwrap();
    let raw = raw.get_or_insert_with(Raw::default);
    if size == 256 {
        let buf = unsafe { std::slice::from_raw_parts_mut(data as *mut u8, 256) };
        for i in 0..256 {
            raw.keys[i] = buf[i] & 0x80 != 0;
        }
        if owns {
            for row in CONTROLS.iter().filter(|r| r.owner != "er") {
                if let Some(k) = dik(row.key) {
                    buf[k] = 0;
                }
            }
        }
        let pass = PASSTHROUGH.lock().unwrap();
        for i in 0..256 {
            if pass[i] {
                buf[i] = 0x80;
            }
        }
    } else if size == 16 || size == 20 {
        let st = unsafe { &mut *(data as *mut DiMouseState2) };
        raw.dx += st.x as f32;
        raw.dy += st.y as f32;
        raw.wheel += st.z as f32;
        let n = if size == 20 { 8 } else { 4 };
        for i in 0..n {
            raw.mouse[i] = st.buttons[i] & 0x80 != 0;
        }
        if owns {
            st.x = 0;
            st.y = 0;
            st.z = 0;
            for b in st.buttons.iter_mut().take(n) {
                *b = 0;
            }
        }
    }
    hr
}

/// Creates a throwaway DirectInput device to find GetDeviceState in its vtable, then hooks it
/// (the game's keyboard and mouse devices share the same implementation).
pub fn install() {
    unsafe {
        let Ok(hinst) = GetModuleHandleW(None) else { return };
        let mut di: Option<IDirectInput8W> = None;
        if DirectInput8Create(hinst.into(), DIRECTINPUT_VERSION, &IDirectInput8W::IID, &mut di as *mut _ as *mut *mut c_void, None).is_err() {
            crate::log!("input: DirectInput8Create failed");
            return;
        }
        let Some(di) = di else { return };
        let mut dev: Option<IDirectInputDevice8W> = None;
        if di.CreateDevice(&GUID_SysKeyboard, &mut dev, None).is_err() {
            crate::log!("input: CreateDevice failed");
            return;
        }
        let Some(dev) = dev else { return };
        let vtable = *(dev.as_raw() as *const *const *mut c_void);
        let target = *vtable.add(9); // IDirectInputDevice8W::GetDeviceState
        if let Some(o) = crate::hook::install("GetDeviceState", target, detour as *mut c_void) {
            ORIGINAL.store(o, Ordering::Release);
        }
        let _ = GUID_SysMouse; // same vtable for the mouse device
    }
}

/// Called once per game frame: takes the input gathered since the last frame.
pub fn frame() -> Frame {
    let mut raw = RAW.lock().unwrap();
    let raw = raw.get_or_insert_with(Raw::default);
    let mut last = LAST.lock().unwrap();
    let prev = last.take().unwrap_or_default();
    let f = Frame {
        keys: raw.keys,
        prev_keys: prev.keys,
        mouse: raw.mouse,
        prev_mouse: prev.mouse,
        dx: std::mem::take(&mut raw.dx),
        dy: std::mem::take(&mut raw.dy),
        wheel: std::mem::take(&mut raw.wheel),
    };
    *last = Some(f.clone());
    f
}

impl Frame {
    fn state(&self, key: &str, prev: bool) -> bool {
        if let Some(b) = key.strip_prefix("MOUSE").and_then(|n| n.parse::<usize>().ok()) {
            let i = b.saturating_sub(1).min(MOUSE_BUTTONS - 1);
            return if prev { self.prev_mouse[i] } else { self.mouse[i] };
        }
        dik(key).map(|k| if prev { self.prev_keys[k] } else { self.keys[k] }).unwrap_or(false)
    }
    pub fn down(&self, control: usize) -> bool {
        self.state(CONTROLS[control].key, false)
    }
    pub fn pressed(&self, control: usize) -> bool {
        self.state(CONTROLS[control].key, false) && !self.state(CONTROLS[control].key, true)
    }
    pub fn released(&self, control: usize) -> bool {
        !self.state(CONTROLS[control].key, false) && self.state(CONTROLS[control].key, true)
    }
}

/// Presses (or releases) Elden Ring's key for a passthrough control this frame.
pub fn passthrough(row: &ControlsRow, on: bool) {
    if let Some(k) = dik(row.er_key) {
        PASSTHROUGH.lock().unwrap()[k] = on;
    }
}

/// DirectInput scan codes for the key names used in sheets/controls.json.
pub fn dik(name: &str) -> Option<usize> {
    Some(match name {
        "ESCAPE" => 0x01, "1" => 0x02, "2" => 0x03, "3" => 0x04, "4" => 0x05, "5" => 0x06, "6" => 0x07,
        "Q" => 0x10, "W" => 0x11, "E" => 0x12, "R" => 0x13, "T" => 0x14, "G" => 0x22, "H" => 0x23,
        "A" => 0x1E, "S" => 0x1F, "D" => 0x20, "F" => 0x21, "Z" => 0x2C, "X" => 0x2D, "C" => 0x2E, "V" => 0x2F,
        "B" => 0x30, "M" => 0x32, "LCONTROL" => 0x1D, "LSHIFT" => 0x2A, "SPACE" => 0x39, "TAB" => 0x0F,
        "F1" => 0x3B, "F2" => 0x3C, "F3" => 0x3D, "F4" => 0x3E, "F5" => 0x3F,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_bound_key_is_known() {
        for r in crate::generated::CONTROLS.iter() {
            assert!(r.key.starts_with("MOUSE") || super::dik(r.key).is_some(), "{}", r.key);
            assert!(r.er_key == "none" || super::dik(r.er_key).is_some(), "{}", r.er_key);
        }
    }
}
