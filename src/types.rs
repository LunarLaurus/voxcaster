// Module-level allow: consumers arrive in later tasks.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Running,
    Killing,
    Killed,
    Exited,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: String,
    pub title: String,
    pub command: String,
    pub args: Vec<String>,
    pub workdir: String,
    pub status: Status,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    pub line_count: usize,
    pub timed_out: bool,
}

#[derive(Debug, Clone)]
pub struct SpawnOptions {
    pub command: String,
    pub args: Vec<String>,
    pub workdir: Option<String>,
    pub env: Vec<(String, String)>,
    pub title: Option<String>,
    pub notify_on_exit: bool,
    pub timeout_seconds: Option<u64>,
}

#[derive(Debug, thiserror::Error)]
pub enum VoxError {
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("denied by policy: {0}")]
    PolicyDenied(String),
    #[error("spawn failed: {0}")]
    Spawn(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_serialises_lowercase() {
        assert_eq!(
            serde_json::to_string(&Status::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&Status::Exited).unwrap(),
            "\"exited\""
        );
    }
}
