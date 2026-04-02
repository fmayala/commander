use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// In-memory handle for a running agent process.
pub struct AgentHandle {
    pub agent_id: String,
    pub task_id: String,
    pub pid: u32,
    pub started_at: Instant,
    pub last_activity: Instant,
    pub restart_count: u32,
}

/// Persistent row in SQLite for agent_id -> pid mapping.
/// Used for startup reconciliation after supervisor crash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRun {
    pub agent_id: String,
    pub task_id: String,
    pub pid: u32,
    /// OS-level process start time (epoch seconds).
    /// Together with pid, forms a unique process identity that survives PID reuse.
    pub proc_start_time: u64,
    pub started_at: DateTime<Utc>,
    pub status: AgentRunStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRunStatus {
    Running,
    Completed,
    Failed,
    Killed,
}

/// Read the OS-level process start time for a given PID.
/// Returns None if the process doesn't exist or can't be queried.
#[cfg(target_os = "macos")]
pub fn proc_start_time(pid: u32) -> Option<u64> {
    use std::mem;
    unsafe {
        let mut info: libc::proc_bsdinfo = mem::zeroed();
        let size = mem::size_of::<libc::proc_bsdinfo>() as i32;
        let ret = libc::proc_pidinfo(
            pid as i32,
            libc::PROC_PIDTBSDINFO,
            0,
            &mut info as *mut _ as *mut libc::c_void,
            size,
        );
        if ret > 0 {
            Some(info.pbi_start_tvsec)
        } else {
            None
        }
    }
}

#[cfg(target_os = "linux")]
pub fn proc_start_time(pid: u32) -> Option<u64> {
    // Read /proc/<pid>/stat, field 22 is starttime in clock ticks
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields: Vec<&str> = stat.split_whitespace().collect();
    let starttime: u64 = fields.get(21)?.parse().ok()?;
    // Convert from clock ticks to seconds (approximate)
    let ticks_per_sec = unsafe { libc::sysconf(libc::_SC_CLK_TCK) } as u64;
    Some(starttime / ticks_per_sec)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn proc_start_time(_pid: u32) -> Option<u64> {
    None
}

/// Check if a process with the given PID is still alive.
pub fn is_pid_alive(pid: u32) -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::kill(pid as i32, 0) == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}
