use std::path::PathBuf;
use std::time::SystemTime;

use serde::Deserialize;

const STALE_SECS: i64 = 120;
const ALERT_FRESH_SECS: i64 = 60;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ClaudeCodeState {
    pub is_working: bool,
    pub is_alert: bool,
    pub is_idle: bool,
    pub is_compacting: bool,
}

impl ClaudeCodeState {
    pub fn idle() -> Self {
        Self {
            is_idle: true,
            ..Self::default()
        }
    }

    /// Lookup used by the state-engine condition evaluator. Matches the input
    /// keys in the pack's edge conditions.
    pub fn get(&self, key: &str) -> Option<bool> {
        match key {
            "claudeCode::isWorking" => Some(self.is_working),
            "claudeCode::isAlert" => Some(self.is_alert),
            "claudeCode::isIdle" => Some(self.is_idle),
            "claudeCode::isCompacting" => Some(self.is_compacting),
            _ => None,
        }
    }
}

/// Per-session JSON written by the Claude Code hooks. Field naming mirrors
/// the original Tauri backend so existing hook scripts keep working. `isIdle`
/// is accepted but never read — idle is derived here from the absence of the
/// other flags, which avoids stale-idle-from-a-session bugs.
#[derive(Debug, Default, Deserialize)]
struct SessionFile {
    #[serde(rename = "claudeCode::isWorking", default)]
    is_working: bool,
    #[serde(rename = "claudeCode::isAlert", default)]
    is_alert: bool,
    #[serde(rename = "claudeCode::isCompacting", default)]
    is_compacting: bool,
}

pub fn sessions_dir() -> PathBuf {
    let mut p = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    p.push("clippy");
    p.push("sessions");
    std::fs::create_dir_all(&p).ok();
    p
}

/// Read all session files under sessions/, drop stale ones, and aggregate
/// their flags with alert > working > compacting > idle priority.
pub fn read_aggregate_state() -> ClaudeCodeState {
    let dir = sessions_dir();
    let now = SystemTime::now();

    let mut any_active = false;
    let mut any_alert = false;
    let mut any_working = false;
    let mut any_compacting = false;

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return ClaudeCodeState::idle();
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let age_secs = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| now.duration_since(t).ok())
            .map(|d| d.as_secs() as i64);
        if matches!(age_secs, Some(a) if a > STALE_SECS) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(s) = serde_json::from_str::<SessionFile>(&text) else {
            continue;
        };
        any_active = true;
        let alert_fresh = age_secs.map(|a| a <= ALERT_FRESH_SECS).unwrap_or(true);
        if s.is_alert && alert_fresh {
            any_alert = true;
        }
        if s.is_working {
            any_working = true;
        }
        if s.is_compacting {
            any_compacting = true;
        }
    }

    let (is_alert, is_working, is_compacting, is_idle) = if any_alert {
        (true, false, false, false)
    } else if any_working {
        (false, true, false, false)
    } else if any_compacting {
        (false, false, true, false)
    } else {
        (false, false, false, true)
    };

    ClaudeCodeState {
        is_working,
        is_alert,
        is_idle: is_idle || !any_active,
        is_compacting,
    }
}
