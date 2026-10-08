//! Where the mod keeps its files on the player's PC.
use std::path::PathBuf;

/// %LOCALAPPDATA%\FortniteRing (Melty's runtimeData; the setup tool writes the cache here).
pub fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(".")).join("FortniteRing")
}

/// Fortnite content converted by the setup tool from the player's own install.
pub fn cache_dir() -> PathBuf {
    data_dir().join("cache")
}

/// Mod-side save data (inventory, opened chests, builds), one file per Elden Ring save slot.
pub fn sidecar(slot: u32) -> PathBuf {
    data_dir().join("saves").join(format!("slot{slot}.json"))
}
