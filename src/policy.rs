// Module-level allow: consumers arrive in later tasks.
#![allow(dead_code)]

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::Deserialize;

use crate::types::VoxError;

#[derive(Debug, Deserialize, Default)]
pub struct PolicyFile {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default = "default_true")]
    pub allow_by_default: bool,
}

fn default_true() -> bool {
    true
}

pub struct PolicyEngine {
    allow: GlobSet,
    deny: GlobSet,
    allow_by_default: bool,
}

impl PolicyEngine {
    pub fn from_lists(allow: Vec<String>, deny: Vec<String>, allow_by_default: bool) -> Self {
        Self {
            allow: build(&allow),
            deny: build(&deny),
            allow_by_default,
        }
    }

    pub fn from_file(path: &str) -> Self {
        let pf: PolicyFile = std::fs::read_to_string(path)
            .ok()
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default();
        Self::from_lists(pf.allow, pf.deny, pf.allow_by_default)
    }

    /// Match against the full "command arg1 arg2" line.
    pub fn check(&self, command: &str, args: &[String]) -> Result<(), VoxError> {
        let line = if args.is_empty() {
            command.to_string()
        } else {
            format!("{} {}", command, args.join(" "))
        };
        if self.deny.is_match(&line) {
            return Err(VoxError::PolicyDenied(line));
        }
        if self.allow_by_default || self.allow.is_match(&line) {
            return Ok(());
        }
        Err(VoxError::PolicyDenied(format!("{line} (not in allowlist)")))
    }
}

/// Build a GlobSet where `*` matches across path separators and spaces.
///
/// By default globset treats `/` as a literal separator — `*` will not cross it.
/// Command lines are not file paths, so we disable that restriction with
/// `literal_separator(false)`, which makes `*` match any character sequence
/// including `/`, spaces, and everything else.
fn build(patterns: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = GlobBuilder::new(p).literal_separator(false).build() {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn engine() -> PolicyEngine {
        PolicyEngine::from_lists(
            vec!["npm *".into(), "cargo *".into()],
            vec!["rm -rf /*".into(), "* | sh".into()],
            true, // allow_by_default
        )
    }

    #[test]
    fn deny_always_wins() {
        let e = engine();
        assert!(e.check("rm", &["-rf".into(), "/var".into()]).is_err());
    }

    #[test]
    fn allowed_command_passes() {
        assert!(engine().check("npm", &["run".into(), "dev".into()]).is_ok());
    }

    #[test]
    fn deny_by_default_blocks_unlisted() {
        let e = PolicyEngine::from_lists(vec!["git *".into()], vec![], false);
        assert!(e.check("curl", &["evil.sh".into()]).is_err());
        assert!(e.check("git", &["status".into()]).is_ok());
    }
}
