//! Physical-memory probe for memory-aware caches and background work
//! (map tile caches, render batch sizing). Best effort: `None` means
//! "unknown" and callers fall back to fixed conservative budgets.

/// System memory snapshot in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemInfo {
    pub total: u64,
    /// Memory the OS can hand out without swapping (Linux `MemAvailable`,
    /// Windows `ullAvailPhys`).
    pub available: u64,
}

impl MemInfo {
    /// Fraction of physical memory still available (0.0–1.0).
    pub fn available_fraction(&self) -> f64 {
        if self.total == 0 {
            return 1.0;
        }
        self.available as f64 / self.total as f64
    }
}

/// Current system memory, when the platform exposes it.
pub fn mem_info() -> Option<MemInfo> {
    imp::mem_info()
}

/// Resident set size of this process in bytes (Linux/Windows; `None`
/// elsewhere).
pub fn process_rss() -> Option<u64> {
    imp::process_rss()
}

#[cfg(target_os = "linux")]
mod imp {
    use super::MemInfo;

    fn field_kb(text: &str, name: &str) -> Option<u64> {
        text.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l[name.len()..].split_whitespace().next())
            .and_then(|v| v.parse::<u64>().ok())
    }

    pub fn mem_info() -> Option<MemInfo> {
        let text = std::fs::read_to_string("/proc/meminfo").ok()?;
        let total = field_kb(&text, "MemTotal:")? * 1024;
        let available = field_kb(&text, "MemAvailable:")
            .or_else(|| Some(field_kb(&text, "MemFree:")? + field_kb(&text, "Cached:").unwrap_or(0)))?
            * 1024;
        // A cgroup limit (containers, systemd scopes) caps what we may use.
        let (total, available) = match cgroup_limit() {
            Some((limit, used)) if limit < total => (limit, available.min(limit.saturating_sub(used))),
            _ => (total, available),
        };
        Some(MemInfo { total, available })
    }

    fn cgroup_limit() -> Option<(u64, u64)> {
        let max = std::fs::read_to_string("/sys/fs/cgroup/memory.max").ok()?;
        let limit = max.trim().parse::<u64>().ok()?;
        let used = std::fs::read_to_string("/sys/fs/cgroup/memory.current")
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);
        Some((limit, used))
    }

    pub fn process_rss() -> Option<u64> {
        let text = std::fs::read_to_string("/proc/self/status").ok()?;
        field_kb(&text, "VmRSS:").map(|kb| kb * 1024)
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use super::MemInfo;

    pub fn mem_info() -> Option<MemInfo> {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let total = sys.total_memory();
        (total > 0).then(|| MemInfo { total, available: sys.available_memory().min(total) })
    }

    pub fn process_rss() -> Option<u64> {
        let pid = sysinfo::get_current_pid().ok()?;
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[pid]), true);
        sys.process(pid).map(sysinfo::Process::memory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_is_sane_when_available() {
        if let Some(m) = mem_info() {
            assert!(m.total > 0);
            assert!(m.available <= m.total);
            let f = m.available_fraction();
            assert!((0.0..=1.0).contains(&f));
        }
    }
}
