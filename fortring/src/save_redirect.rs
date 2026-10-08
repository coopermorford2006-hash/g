//! systems:save_redirect. Elden Ring saves to %APPDATA%\EldenRing\<id>\ER0000.sl2. While Fortnite Ring
//! runs, every open of ER0000.sl2(.bak) goes to ER0000.fnring.sl2(.bak) instead, so the player's normal
//! save never sees mod items and the normal online game is untouched.
use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::core::{PCWSTR, s, w};

type CreateFileW = unsafe extern "system" fn(PCWSTR, u32, u32, *const c_void, u32, u32, HANDLE) -> HANDLE;
static ORIGINAL: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

pub const SAVE_SUFFIX: &str = ".fnring";

fn redirect(path: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    if !lower.contains("eldenring") {
        return None;
    }
    for ext in [".sl2.bak", ".sl2"] {
        if lower.ends_with(ext) && !lower.ends_with(&format!("{SAVE_SUFFIX}{ext}")) {
            let stem = &path[..path.len() - ext.len()];
            return Some(format!("{stem}{SAVE_SUFFIX}{ext}"));
        }
    }
    None
}

unsafe extern "system" fn detour(
    name: PCWSTR, access: u32, share: u32, sa: *const c_void, disp: u32, flags: u32, template: HANDLE,
) -> HANDLE {
    let orig: CreateFileW = unsafe { std::mem::transmute(ORIGINAL.load(Ordering::Acquire)) };
    if !name.is_null() {
        if let Ok(path) = unsafe { name.to_string() } {
            if let Some(new) = redirect(&path) {
                let wide: Vec<u16> = new.encode_utf16().chain(std::iter::once(0)).collect();
                return unsafe { orig(PCWSTR(wide.as_ptr()), access, share, sa, disp, flags, template) };
            }
        }
    }
    unsafe { orig(name, access, share, sa, disp, flags, template) }
}

pub fn install() {
    unsafe {
        let Ok(k) = GetModuleHandleW(w!("kernelbase.dll")) else { return };
        let Some(target) = GetProcAddress(k, s!("CreateFileW")) else { return };
        if let Some(orig) = crate::hook::install("CreateFileW", target as *mut c_void, detour as *mut c_void) {
            ORIGINAL.store(orig, Ordering::Release);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::redirect;
    #[test]
    fn redirects_only_elden_ring_saves() {
        assert_eq!(redirect(r"C:\Users\a\AppData\Roaming\EldenRing\7656\ER0000.sl2").as_deref(),
                   Some(r"C:\Users\a\AppData\Roaming\EldenRing\7656\ER0000.fnring.sl2"));
        assert_eq!(redirect(r"C:\x\EldenRing\1\ER0000.sl2.bak").as_deref(), Some(r"C:\x\EldenRing\1\ER0000.fnring.sl2.bak"));
        assert_eq!(redirect(r"C:\x\EldenRing\1\ER0000.fnring.sl2"), None);
        assert_eq!(redirect(r"C:\x\Other\ER0000.sl2"), None);
    }
}
