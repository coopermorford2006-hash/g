//! systems:assets. The Fortnite content cache made by the setup tool (FortniteRingSetup.exe) from the
//! player's own Fortnite install. manifest.json lists what was found; the DLL never ships Fortnite files.
use std::sync::atomic::{AtomicBool, Ordering};

static READY: AtomicBool = AtomicBool::new(false);

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
            READY.store(m.missing_required.is_empty(), Ordering::Relaxed);
        }
        None => crate::log!("assets: no cache at {} (the setup tool has not run)", path.display()),
    }
}

pub fn ready() -> bool {
    READY.load(Ordering::Relaxed)
}
