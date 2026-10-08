//! FortniteRing.log: the oracle for every in-game test (systems:log).
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;
use windows::Win32::System::Diagnostics::Debug::{AddVectoredExceptionHandler, EXCEPTION_POINTERS};

static LOG: Mutex<Option<(File, Instant)>> = Mutex::new(None);

pub fn init() {
    let dir = crate::paths::data_dir();
    let _ = std::fs::create_dir_all(&dir);
    // a crashed game still being torn down by Windows Error Reporting can keep the old log locked:
    // fall back to a per-process file rather than running the whole session without a log
    let open = |name: String| OpenOptions::new().create(true).write(true).truncate(true).open(dir.join(name));
    let file = open("FortniteRing.log".into()).or_else(|_| open(format!("FortniteRing-{}.log", std::process::id())));
    if let Ok(f) = file {
        *LOG.lock().unwrap() = Some((f, Instant::now()));
    }
    // panic = "abort" kills the game; record why first (stderr goes nowhere inside eldenring.exe)
    std::panic::set_hook(Box::new(|info| {
        write(&format!("PANIC: {info}\n{}", std::backtrace::Backtrace::force_capture()));
    }));
    // crashes that never reach the panic hook (access violations, abort() from C code): log where they happened
    unsafe { AddVectoredExceptionHandler(0, Some(on_exception)) };
}

/// Last in the handler chain, so exceptions the game handles itself mostly never get here. Logs the first
/// few fatal-looking ones with a backtrace (symbolized when fortring.pdb sits next to the DLL), then lets
/// the exception continue as if the handler were not there.
unsafe extern "system" fn on_exception(info: *mut EXCEPTION_POINTERS) -> i32 {
    static SEEN: AtomicU32 = AtomicU32::new(0);
    let rec = unsafe { &*(*info).ExceptionRecord };
    let code = rec.ExceptionCode.0 as u32;
    // breakpoint (abort), access violation, stack buffer overrun / fast fail, illegal instruction
    if matches!(code, 0x8000_0003 | 0xC000_0005 | 0xC000_0409 | 0xC000_001D) && SEEN.fetch_add(1, Ordering::Relaxed) < 8 {
        let addr = rec.ExceptionAddress as usize;
        let base = unsafe { crate::MODULE };
        let params = &rec.ExceptionInformation[..(rec.NumberParameters as usize).min(4)];
        let msg = format!(
            "EXCEPTION {code:#010x} at {addr:#x} (fortring.dll+{:#x}) params {params:x?} thread {:?}\n{}",
            addr.wrapping_sub(base),
            std::thread::current().id(),
            std::backtrace::Backtrace::force_capture()
        );
        // try_lock: the faulting thread may already hold the log
        if let Ok(mut g) = LOG.try_lock() {
            if let Some((f, t0)) = g.as_mut() {
                let _ = writeln!(f, "[{:9.3}] {msg}", t0.elapsed().as_secs_f64());
                let _ = f.flush();
            }
        }
    }
    0 // EXCEPTION_CONTINUE_SEARCH
}

pub fn write(msg: &str) {
    if let Ok(mut g) = LOG.lock() {
        if let Some((f, t0)) = g.as_mut() {
            let _ = writeln!(f, "[{:9.3}] {msg}", t0.elapsed().as_secs_f64());
            let _ = f.flush();
        }
    }
}

#[macro_export]
macro_rules! log {
    ($($t:tt)*) => { $crate::log::write(&format!($($t)*)) };
}
