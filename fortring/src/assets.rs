//! systems:assets. The Fortnite content cache made by the setup tool (FortniteRingSetup.exe) from the
//! player's own Fortnite install. manifest.json lists what was found; the DLL never ships Fortnite files.
use std::sync::atomic::{AtomicI32, Ordering};

/// Required assets the setup tool could not convert; -1 = no cache at all (the setup tool never ran).
static MISSING: AtomicI32 = AtomicI32::new(-1);

#[derive(serde::Deserialize)]
struct Manifest {
    fortnite_build: String,
    found: Vec<String>,
    missing_required: Vec<String>,
}

pub fn load() {
    let path = crate::paths::cache_dir().join("manifest.json");
    match std::fs::read(&path).ok().and_then(|b| serde_json::from_slice::<Manifest>(&b).ok()) {
        Some(m) => {
            crate::log!("assets: Fortnite {} cache, {} assets, {} required missing {:?}", m.fortnite_build, m.found.len(), m.missing_required.len(), m.missing_required);
            MISSING.store(m.missing_required.len() as i32, Ordering::Relaxed);
        }
        None => crate::log!("assets: no cache at {} (the setup tool has not run)", path.display()),
    }
}

/// None = the setup tool has not run; Some(n) = n required assets are missing (0 = all there).
pub fn missing() -> Option<u32> {
    u32::try_from(MISSING.load(Ordering::Relaxed)).ok()
}
