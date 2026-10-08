//! Where the mod keeps its files on the player's PC.
use std::path::PathBuf;
use std::sync::OnceLock;

/// The user's Local AppData folder from Windows itself (SHGetKnownFolderPath, as the setup tool's
/// Environment.GetFolderPath does). The LOCALAPPDATA variable is only a fallback: the shell that starts
/// ModEngine2 may pass a different or missing one, which hid the cache and the log from the DLL.
fn local_app_data() -> PathBuf {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath};
    if let Ok(p) = unsafe { SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None) } {
        let s = unsafe { p.to_string() };
        unsafe { CoTaskMemFree(Some(p.0 as *const _)) };
        if let Ok(s) = s {
            return PathBuf::from(s);
        }
    }
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// %LOCALAPPDATA%\FortniteRing (Melty's runtimeData; the setup tool writes the cache here).
pub fn data_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| local_app_data().join("FortniteRing")).clone()
}

/// Fortnite content converted by the setup tool from the player's own install.
pub fn cache_dir() -> PathBuf {
    data_dir().join("cache")
}

/// Mod-side save data (inventory, opened chests, builds), one file per Elden Ring save slot.
pub fn sidecar(slot: u32) -> PathBuf {
    data_dir().join("saves").join(format!("slot{slot}.json"))
}
