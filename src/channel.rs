#![allow(dead_code)]

use crate::types::SessionInfo;
use serde_json::{json, Value};

pub const CHANNEL_METHOD: &str = "notifications/claude/channel";

/// Build the params for a `notifications/claude/channel` exit push.
/// Renders in-session as `<channel source="voxcaster" session_id=.. exit_code=..>content</channel>`.
pub fn build_exit_params(session_id: &str, cmdline: &str, exit_code: Option<i32>) -> Value {
    let code = exit_code
        .map(|c| c.to_string())
        .unwrap_or_else(|| "unknown".into());
    json!({
        "content": format!("Process `{cmdline}` exited (code {code})."),
        "meta": {
            "session_id": session_id,
            "exit_code": code
        }
    })
}

/// Build exit-push params from a finished session's info.
pub fn exit_params_for(info: &SessionInfo) -> Value {
    let cmdline = format!("{} {}", info.command, info.args.join(" "))
        .trim()
        .to_string();
    build_exit_params(&info.id, &cmdline, info.exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn builds_exit_notification_params() {
        let v = build_exit_params("pty_abcd", "npm run dev", Some(0));
        assert_eq!(v["meta"]["session_id"], "pty_abcd");
        assert_eq!(v["meta"]["exit_code"], "0");
        assert!(v["content"].as_str().unwrap().contains("npm run dev"));
    }
}
