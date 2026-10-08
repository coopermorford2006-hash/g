//! Small wrapper over MinHook (bundled in hudhook) for function hooks.
use hudhook::mh::{MH_CreateHook, MH_EnableHook, MH_Initialize, MH_STATUS};
use std::ffi::c_void;

pub fn init() {
    // hudhook initialises MinHook too; MH_ERROR_ALREADY_INITIALIZED is fine.
    let _ = unsafe { MH_Initialize() };
}

/// Hooks `target` with `detour`; returns the trampoline to call the original.
pub unsafe fn install(name: &str, target: *mut c_void, detour: *mut c_void) -> Option<*mut c_void> {
    let mut original: *mut c_void = std::ptr::null_mut();
    let st = unsafe { MH_CreateHook(target, detour, &mut original) };
    if st != MH_STATUS::MH_OK {
        crate::log!("hook {name}: create failed {st:?}");
        return None;
    }
    let st = unsafe { MH_EnableHook(target) };
    if st != MH_STATUS::MH_OK {
        crate::log!("hook {name}: enable failed {st:?}");
        return None;
    }
    crate::log!("hook {name}: on");
    Some(original)
}
