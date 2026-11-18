//! **OMT patch** — pointer lock + raw mouse motion for the 3D viewer's
//! game-style mouse-look (iced 0.14 exposes neither). The app requests a
//! lock / unlock; the event loop applies it to the windows on its next
//! iteration (hidden cursor, `CursorGrabMode::Locked`, falling back to
//! `Confined` where locking is unsupported, e.g. X11). Relative motion
//! comes from winit's `DeviceEvent::MouseMotion`, which keeps flowing
//! while the pointer is locked, and is accumulated until the app takes it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

static REQUEST: AtomicU8 = AtomicU8::new(0);
static LOCKED: AtomicBool = AtomicBool::new(false);
static MOTION: Mutex<(f64, f64)> = Mutex::new((0.0, 0.0));

/// Ask for the pointer to be locked (`true`) or released (`false`).
pub fn request(lock: bool) {
    REQUEST.store(if lock { 1 } else { 2 }, Ordering::SeqCst);
}

/// Whether a lock (or confinement) is currently in effect.
pub fn locked() -> bool {
    LOCKED.load(Ordering::SeqCst)
}

/// Relative motion accumulated since the last call (physical pixels).
pub fn take_motion() -> (f64, f64) {
    std::mem::take(&mut *MOTION.lock().unwrap_or_else(std::sync::PoisonError::into_inner))
}

pub(crate) fn add_motion(dx: f64, dy: f64) {
    if locked() {
        let mut m = MOTION.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        m.0 += dx;
        m.1 += dy;
    }
}

/// Apply a pending request to the windows.
pub(crate) fn apply<'a>(windows: impl Iterator<Item = &'a winit::window::Window>) {
    use winit::window::CursorGrabMode;
    match REQUEST.swap(0, Ordering::SeqCst) {
        1 => {
            let mut ok = false;
            for w in windows {
                let grabbed = w.set_cursor_grab(CursorGrabMode::Locked).or_else(|_| w.set_cursor_grab(CursorGrabMode::Confined)).is_ok();
                if grabbed {
                    w.set_cursor_visible(false);
                }
                ok |= grabbed;
            }
            let _ = take_motion();
            LOCKED.store(ok, Ordering::SeqCst);
        }
        2 => {
            for w in windows {
                let _ = w.set_cursor_grab(CursorGrabMode::None);
                w.set_cursor_visible(true);
            }
            LOCKED.store(false, Ordering::SeqCst);
            let _ = take_motion();
        }
        _ => {}
    }
}
