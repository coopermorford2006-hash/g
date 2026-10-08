//! systems:safety. The mod only runs offline, started by Melty through ModEngine2 with Easy Anti-Cheat off.
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::core::w;

pub fn offline_and_safe() -> Result<(), &'static str> {
    // start_protected_game.exe loads the EasyAntiCheat module into the game; ModEngine2 starts eldenring.exe directly.
    let eac = unsafe { GetModuleHandleW(w!("EasyAntiCheat_EOS.dll")) }.is_ok()
        || unsafe { GetModuleHandleW(w!("EasyAntiCheat.dll")) }.is_ok();
    if eac {
        return Err("Easy Anti-Cheat is loaded: Fortnite Ring stays off (start it from Melty)");
    }
    Ok(())
}
