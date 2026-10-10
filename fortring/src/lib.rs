//! Fortnite Ring: Elden Ring played as Fortnite. Loaded by ModEngine2 (external_dlls), which Melty
//! installs and starts offline with Easy Anti-Cheat off. See sheets/systems.json for every system.
mod assets;
mod audio;
mod building;
mod camera;
mod generated;
mod guns;
mod harvest;
mod health;
mod hook;
mod hud;
mod input;
mod log;
mod model;
mod loot;
mod movement;
mod paths;
mod safety;
mod save_redirect;
mod state;
mod tick;
mod world;

use eldenring::cs::{CSTaskGroupIndex, CSTaskImp};
use eldenring::fd4::FD4TaskData;
use fromsoftware_shared::{FromStatic as _, SharedTaskImpExt};
use hudhook::Hudhook;
use hudhook::hooks::dx12::ImguiDx12Hooks;
use std::time::Duration;
use hudhook::windows::Win32::Foundation::HINSTANCE as HudInstance;
use windows::Win32::Foundation::HINSTANCE;

static mut MODULE: usize = 0;

#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllMain(module: HINSTANCE, reason: u32, _: *mut core::ffi::c_void) -> i32 {
    if reason != 1 {
        return 1;
    }
    unsafe { MODULE = module.0 as usize };
    log::init();
    log!("Fortnite Ring {} loading", env!("CARGO_PKG_VERSION"));
    log!("data folder {} (LOCALAPPDATA={:?})", paths::data_dir().display(), std::env::var_os("LOCALAPPDATA"));
    if let Err(why) = safety::offline_and_safe() {
        log!("{why}");
        return 1;
    }
    hook::init();
    // before the game opens its save files
    save_redirect::install();
    std::thread::spawn(boot);
    1
}

/// systems:boot. Waits for the game's task system, then registers the per-frame work.
fn boot() {
    let task = match CSTaskImp::wait_for_instance(Duration::from_secs(600)) {
        Ok(t) => t,
        Err(e) => {
            log!("boot: CSTask never appeared: {e:?}");
            return;
        }
    };
    log!("boot: game task system up");
    input::install();
    guns::install_spawn_hook();
    audio::init();
    assets::load();

    let rows_ready = std::sync::atomic::AtomicBool::new(false);
    task.run_recurring(
        move |_: &FD4TaskData| {
            if !rows_ready.load(std::sync::atomic::Ordering::Relaxed) && world::params().is_some() && world::player().is_some() {
                guns::pick_rows();
                loot::load_graces();
                blank_you_died();
                rows_ready.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            tick::gameplay();
        },
        CSTaskGroupIndex::ChrIns_PostPhysics,
    );
    task.run_recurring(|_: &FD4TaskData| tick::camera_write(), CSTaskGroupIndex::Draw_Pre);

    let hinst = HudInstance(unsafe { MODULE } as *mut core::ffi::c_void);
    if let Err(e) = Hudhook::builder().with::<ImguiDx12Hooks>(hud::Hud::new()).with_hmodule(hinst).build().apply() {
        log!("boot: HUD hook failed {e:?}");
    }
    log!("boot: running");
}

/// FeTextEffectParam rows 5 and 50 are the YOU DIED banner (er_rows.you_died_banner).
fn blank_you_died() {
    use eldenring::cs::FeTextEffectParam;
    let Some(params) = world::params() else { return };
    for &id in generated::ER_ROWS[generated::ER_ROWS_YOU_DIED_BANNER].candidates {
        if let Some(r) = params.get_mut::<FeTextEffectParam>(id as u32) {
            r.set_res_id(0);
            r.set_text_id(-1);
            r.set_se_id(-1);
            log!("er_rows: FeTextEffectParam {id} blanked");
        }
    }
}
