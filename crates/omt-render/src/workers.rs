//! The map renderer's CPU budget (uNmINeD's "chunk processors" picker):
//! tile builds run inside a dedicated rayon pool whose size the user
//! chooses (`0` = every core). Nested parallel work (region decoding,
//! Bedrock batched reads) runs on the same pool, so the setting caps the
//! renderer's whole CPU use.

use std::sync::{Arc, OnceLock, RwLock};

/// The current pool and its size.
type Slot = RwLock<Option<(usize, Arc<rayon::ThreadPool>)>>;

fn slot() -> &'static Slot {
    static S: OnceLock<Slot> = OnceLock::new();
    S.get_or_init(|| RwLock::new(None))
}

/// Logical CPUs on this machine.
pub fn cpus() -> usize {
    std::thread::available_parallelism().map_or(4, |n| n.get())
}

/// Threads for a setting (`0` = all CPUs).
pub fn effective(threads: usize) -> usize {
    if threads == 0 { cpus() } else { threads.min(cpus().max(1) * 2) }
}

/// The picker's choices (uNmINeD-style steps: 1–12, then by 2 to 24, then
/// by 4), capped at this machine's CPUs; `0` = "Max CPU".
pub fn choices() -> Vec<usize> {
    let max = cpus();
    let mut v: Vec<usize> = (1..=12).chain((14..=24).step_by(2)).chain((28..=256).step_by(4)).filter(|&n| n < max).collect();
    v.push(0);
    v
}

/// Run `f` on the renderer pool sized for `threads` (rebuilt on change).
pub fn install<R: Send>(threads: usize, f: impl FnOnce() -> R + Send) -> R {
    let n = effective(threads);
    let pool = {
        let cur = slot().read().expect("worker pool");
        cur.as_ref().filter(|(size, _)| *size == n).map(|(_, p)| Arc::clone(p))
    };
    let pool = match pool {
        Some(p) => p,
        None => {
            let p = Arc::new(
                rayon::ThreadPoolBuilder::new()
                    .num_threads(n)
                    .thread_name(|i| format!("omt-render-{i}"))
                    .build()
                    .expect("render thread pool"),
            );
            *slot().write().expect("worker pool") = Some((n, Arc::clone(&p)));
            p
        }
    };
    pool.install(f)
}

#[cfg(test)]
mod tests {
    #[test]
    fn pool_size_follows_setting() {
        assert_eq!(super::install(2, rayon::current_num_threads), 2);
        assert_eq!(super::install(0, rayon::current_num_threads), super::cpus());
        assert_eq!(*super::choices().last().unwrap(), 0);
    }
}
