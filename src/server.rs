//! MCP server surface for voxcaster.
//!
//! Exposes the [`PtyManager`] as six MCP tools over stdio via the `rmcp` SDK
//! (verified against rmcp 1.7.0). Each tool accepts an optional
//! `format: "text" | "json"` (default `text`). The wire-facing tool handlers
//! are thin wrappers around inner methods (`*_inner`) that carry no rmcp types,
//! so the core logic stays unit-testable without a transport.

use std::collections::HashMap;
use std::sync::Arc;

use globset::Glob;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    CallToolResult, Content, CustomNotification, ServerCapabilities, ServerInfo, ServerNotification,
};
use rmcp::service::{Peer, RoleServer};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use crate::policy::PolicyEngine;
use crate::pty::manager::PtyManager;
use crate::types::{SpawnOptions, VoxError};

/// The voxcaster MCP server: owns the PTY manager and the command policy.
#[derive(Clone)]
pub struct VoxServer {
    manager: PtyManager,
    policy: Arc<PolicyEngine>,
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl VoxServer {
    /// Construct a server over the given PTY manager and policy engine.
    pub fn new(manager: PtyManager, policy: Arc<PolicyEngine>) -> Self {
        Self {
            manager,
            policy,
            tool_router: Self::tool_router(),
        }
    }

    // ---- Inner methods: no rmcp types, directly unit-testable ----

    /// Check policy, then spawn a session. Returns the new session id.
    pub fn spawn_inner(&self, opts: SpawnOptions) -> Result<String, VoxError> {
        self.policy.check(&opts.command, &opts.args)?;
        self.manager.spawn(opts)
    }

    /// Write `data` to a session's stdin.
    pub fn write_inner(&self, id: &str, data: &str) -> Result<(), VoxError> {
        self.manager.write(id, data)
    }

    /// Block until the session exits (or the optional timeout elapses).
    pub async fn wait_inner(&self, id: &str, timeout_seconds: Option<u64>) -> Result<(), VoxError> {
        self.manager.wait(id, timeout_seconds).await.map(|_| ())
    }

    /// Read a slice of a session's scrollback, joined with newlines.
    ///
    /// When `pattern` is `Some`, only lines matching the glob are returned.
    /// An unparseable glob yields no matches rather than an error.
    pub fn read_inner(
        &self,
        id: &str,
        offset: usize,
        limit: Option<usize>,
        raw: bool,
        pattern: Option<&str>,
    ) -> Result<String, VoxError> {
        let slice = self.manager.read(id, offset, limit, raw)?;
        let lines = filter_lines(slice.lines, pattern);
        Ok(lines.join("\n"))
    }

    /// List all known sessions.
    pub fn list_inner(&self) -> Vec<crate::types::SessionInfo> {
        self.manager.list()
    }

    /// Kill a session, optionally removing it from the registry.
    pub fn kill_inner(&self, id: &str, cleanup: bool) -> Result<(), VoxError> {
        self.manager.kill(id, cleanup)
    }

    /// Spawn a detached task that, when session `id` exits, pushes a
    /// `notifications/claude/channel` notification to `peer`.
    ///
    /// This is the research-preview "channels" enhancement: the blocking
    /// `pty_wait` tool is the guaranteed floor. Send failures (no active
    /// channel, disconnected client) are swallowed — they must never crash
    /// the session. Errors are logged to stderr only; stdout is reserved for
    /// the MCP transport.
    fn spawn_exit_push(&self, peer: Peer<RoleServer>, id: String) {
        let manager = self.manager.clone();
        tokio::spawn(async move {
            if let Ok(info) = manager.wait(&id, None).await {
                let params = crate::channel::exit_params_for(&info);
                let notification = ServerNotification::CustomNotification(CustomNotification::new(
                    crate::channel::CHANNEL_METHOD,
                    Some(params),
                ));
                if let Err(e) = peer.send_notification(notification).await {
                    eprintln!("voxcaster: channel exit-push for session {id} failed: {e}");
                }
            }
        });
    }
}

/// Apply an optional glob filter to a set of lines.
///
/// A `None` pattern returns the lines unchanged. A pattern that fails to
/// compile matches nothing (conservative: do not surface a bad filter as data).
fn filter_lines(lines: Vec<String>, pattern: Option<&str>) -> Vec<String> {
    match pattern {
        None => lines,
        Some(p) => match Glob::new(p) {
            Ok(g) => {
                let m = g.compile_matcher();
                lines.into_iter().filter(|l| m.is_match(l)).collect()
            }
            Err(_) => Vec::new(),
        },
    }
}

/// Normalise the optional `format` field to `"json"` vs anything-else (text).
fn is_json(format: &Option<String>) -> bool {
    matches!(format.as_deref(), Some("json"))
}

fn default_false() -> bool {
    false
}

// ---- Tool parameter structs ----

/// Parameters for `pty_spawn`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct SpawnParams {
    /// Executable to launch.
    pub command: String,
    /// Arguments passed to the executable.
    #[serde(default)]
    pub args: Vec<String>,
    /// Working directory for the new process.
    #[serde(default)]
    pub workdir: Option<String>,
    /// Extra environment variables.
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Human-readable session title.
    #[serde(default)]
    pub title: Option<String>,
    /// Optional free-text description (informational only).
    #[serde(default)]
    pub description: Option<String>,
    /// Push a `claude/channel` notification when the process exits.
    #[serde(default = "default_false")]
    pub notify_on_exit: bool,
    /// Auto-kill the session after this many seconds.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    /// Output rendering: `"text"` (default) or `"json"`.
    #[serde(default)]
    pub format: Option<String>,
}

/// Parameters for `pty_write`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WriteParams {
    /// Target session id.
    pub id: String,
    /// Raw bytes to write to the session's stdin.
    pub data: String,
    /// Output rendering: `"text"` (default) or `"json"`.
    #[serde(default)]
    pub format: Option<String>,
}

/// Parameters for `pty_read`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadParams {
    /// Target session id.
    pub id: String,
    /// Line offset to start reading from.
    #[serde(default)]
    pub offset: usize,
    /// Maximum number of lines to return.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Return raw output including ANSI escapes when `true`.
    #[serde(default = "default_false")]
    pub raw: bool,
    /// Optional glob to filter returned lines.
    #[serde(default)]
    pub pattern: Option<String>,
    /// Output rendering: `"text"` (default) or `"json"`.
    #[serde(default)]
    pub format: Option<String>,
}

/// Parameters for `pty_wait`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct WaitParams {
    /// Target session id.
    pub id: String,
    /// Maximum seconds to block before returning.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    /// Output rendering: `"text"` (default) or `"json"`.
    #[serde(default)]
    pub format: Option<String>,
}

/// Parameters for `pty_list`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListParams {
    /// Output rendering: `"text"` (default) or `"json"`.
    #[serde(default)]
    pub format: Option<String>,
}

/// Parameters for `pty_kill`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct KillParams {
    /// Target session id.
    pub id: String,
    /// Remove the session from the registry after killing.
    #[serde(default = "default_false")]
    pub cleanup: bool,
    /// Output rendering: `"text"` (default) or `"json"`.
    #[serde(default)]
    pub format: Option<String>,
}

// ---- Tool surface ----

#[tool_router]
impl VoxServer {
    /// Spawn a new interactive PTY session and return its id.
    #[tool(
        name = "pty_spawn",
        description = "Spawn a new interactive PTY session running a command."
    )]
    async fn pty_spawn(
        &self,
        Parameters(p): Parameters<SpawnParams>,
        peer: Peer<RoleServer>,
    ) -> CallToolResult {
        let json_out = is_json(&p.format);
        let notify_on_exit = p.notify_on_exit;
        let opts = SpawnOptions {
            command: p.command,
            args: p.args,
            workdir: p.workdir,
            env: p.env.into_iter().collect(),
            title: p.title,
            notify_on_exit,
            timeout_seconds: p.timeout_seconds,
        };
        match self.spawn_inner(opts) {
            Ok(id) => {
                if notify_on_exit {
                    self.spawn_exit_push(peer, id.clone());
                }
                let v = json!({ "sessionId": id });
                if json_out {
                    success_json(v)
                } else {
                    render_text(v)
                }
            }
            Err(e) => err_result(e),
        }
    }

    /// Write data to a session's stdin.
    #[tool(name = "pty_write", description = "Write data to a session's stdin.")]
    async fn pty_write(&self, Parameters(p): Parameters<WriteParams>) -> CallToolResult {
        let json_out = is_json(&p.format);
        match self.write_inner(&p.id, &p.data) {
            Ok(()) => {
                let v = json!({ "sessionId": p.id, "written": true });
                if json_out {
                    success_json(v)
                } else {
                    render_text(v)
                }
            }
            Err(e) => err_result(e),
        }
    }

    /// Read a slice of a session's scrollback buffer.
    #[tool(
        name = "pty_read",
        description = "Read a line range from a session's scrollback buffer."
    )]
    async fn pty_read(&self, Parameters(p): Parameters<ReadParams>) -> CallToolResult {
        let json_out = is_json(&p.format);
        // For JSON output we need the structured slice; read it directly.
        match self.manager.read(&p.id, p.offset, p.limit, p.raw) {
            Ok(slice) => {
                let lines = filter_lines(slice.lines, p.pattern.as_deref());
                if json_out {
                    success_json(json!({
                        "sessionId": p.id,
                        "lineRange": {
                            "offset": slice.offset,
                            "limit": p.limit,
                            "total": slice.total,
                        },
                        "lines": lines,
                        "truncated": slice.truncated,
                    }))
                } else {
                    CallToolResult::success(vec![Content::text(lines.join("\n"))])
                }
            }
            Err(e) => err_result(e),
        }
    }

    /// Block until a session exits, then report its final state.
    #[tool(
        name = "pty_wait",
        description = "Block until a session exits or the timeout elapses."
    )]
    async fn pty_wait(&self, Parameters(p): Parameters<WaitParams>) -> CallToolResult {
        let json_out = is_json(&p.format);
        match self.manager.wait(&p.id, p.timeout_seconds).await {
            Ok(info) => match serde_json::to_value(&info) {
                Ok(v) => {
                    if json_out {
                        success_json(v)
                    } else {
                        render_text(v)
                    }
                }
                Err(e) => CallToolResult::error(vec![Content::text(format!("{e}"))]),
            },
            Err(e) => err_result(e),
        }
    }

    /// List all known sessions.
    #[tool(name = "pty_list", description = "List all known PTY sessions.")]
    async fn pty_list(&self, Parameters(p): Parameters<ListParams>) -> CallToolResult {
        let json_out = is_json(&p.format);
        let sessions = self.list_inner();
        match serde_json::to_value(&sessions) {
            Ok(v) => {
                if json_out {
                    success_json(json!({ "sessions": v }))
                } else {
                    render_text(v)
                }
            }
            Err(e) => CallToolResult::error(vec![Content::text(format!("{e}"))]),
        }
    }

    /// Kill a session, optionally removing it from the registry.
    #[tool(
        name = "pty_kill",
        description = "Kill a session and optionally remove it from the registry."
    )]
    async fn pty_kill(&self, Parameters(p): Parameters<KillParams>) -> CallToolResult {
        let json_out = is_json(&p.format);
        match self.kill_inner(&p.id, p.cleanup) {
            Ok(()) => {
                let v = json!({ "sessionId": p.id, "killed": true, "cleanup": p.cleanup });
                if json_out {
                    success_json(v)
                } else {
                    render_text(v)
                }
            }
            Err(e) => err_result(e),
        }
    }
}

/// Render a [`serde_json::Value`] as concise human-readable text.
///
/// - Object → one `key: value` line per top-level field; nested objects/arrays
///   are compacted to their JSON representation.
/// - Array → each element rendered with `human()`, joined by newlines.
/// - String → the string value (no surrounding quotes).
/// - Number/Bool/Null → their display representation.
fn human(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, val)| {
                let rendered = match val {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("{k}: {rendered}")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        serde_json::Value::Array(arr) => arr.iter().map(human).collect::<Vec<_>>().join("\n"),
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Render a `serde_json::Value` envelope as a single JSON text content block.
fn success_json(value: serde_json::Value) -> CallToolResult {
    let text = serde_json::to_string(&value).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"));
    CallToolResult::success(vec![Content::text(text)])
}

/// Render a `serde_json::Value` as concise human-readable text via [`human`].
fn render_text(value: serde_json::Value) -> CallToolResult {
    CallToolResult::success(vec![Content::text(human(&value))])
}

/// Render a [`VoxError`] as an error tool result carrying its display string.
fn err_result(e: VoxError) -> CallToolResult {
    CallToolResult::error(vec![Content::text(format!("{e}"))])
}

#[tool_handler]
impl ServerHandler for VoxServer {
    fn get_info(&self) -> ServerInfo {
        // Advertise tools, and declare the `claude/channel` experimental capability
        // by its literal key. Claude Code only registers this server as a channel
        // (and delivers our notifications/claude/channel exit-pushes) when the key
        // is present; an empty `experimental: {}` map is silently treated as no
        // channel. The required value is an empty JSON object (`{}`).
        let mut capabilities = ServerCapabilities::builder().enable_tools().build();
        let mut experimental = std::collections::BTreeMap::new();
        experimental.insert("claude/channel".to_string(), serde_json::Map::new());
        capabilities.experimental = Some(experimental);

        // `ServerInfo` (alias for `InitializeResult`) is `#[non_exhaustive]`, so
        // construct via `ServerInfo::new(..)` and set fields with public setters.
        let mut info = ServerInfo::new(capabilities);
        info.instructions = Some(
            "voxcaster exposes persistent interactive PTY sessions to an agent. \
             Use pty_spawn to start a process, pty_write to send input, pty_read to \
             inspect scrollback, pty_wait to block on exit, pty_list to enumerate \
             sessions, and pty_kill to terminate one. Each tool accepts an optional \
             `format` of \"text\" or \"json\"."
                .to_string(),
        );
        info
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_then_read_and_policy_denies() {
        let policy = std::sync::Arc::new(crate::policy::PolicyEngine::from_lists(
            vec![],
            vec![],
            true,
        ));
        let srv = VoxServer::new(crate::pty::manager::PtyManager::new(), policy);

        #[cfg(windows)]
        let (command, args) = (
            "cmd".to_string(),
            vec!["/C".to_string(), "echo hi".to_string()],
        );
        #[cfg(not(windows))]
        let (command, args) = ("/bin/echo".to_string(), vec!["hi".to_string()]);

        let id = srv
            .spawn_inner(crate::types::SpawnOptions {
                command,
                args,
                workdir: None,
                env: vec![],
                title: None,
                notify_on_exit: false,
                timeout_seconds: None,
            })
            .unwrap();
        srv.wait_inner(&id, Some(10)).await.unwrap();
        let txt = srv.read_inner(&id, 0, None, false, None).unwrap();
        assert!(txt.contains("hi"), "got: {txt}");

        let denied = crate::policy::PolicyEngine::from_lists(vec![], vec!["rm *".into()], true);
        assert!(denied.check("rm", &["x".into()]).is_err());
    }

    #[test]
    fn glob_filter_keeps_matching_lines() {
        let lines = vec!["error: boom".to_string(), "info: ok".to_string()];
        let kept = filter_lines(lines, Some("error:*"));
        assert_eq!(kept, vec!["error: boom"]);
    }

    #[test]
    fn bad_glob_matches_nothing() {
        let lines = vec!["a".to_string()];
        assert!(filter_lines(lines, Some("[")).is_empty());
    }

    #[test]
    fn human_renders_object_as_keyvalue() {
        let s = human(&serde_json::json!({"id": "pty_x", "status": "running"}));
        assert!(s.contains("id: pty_x"), "got: {s}");
        assert!(s.contains("status: running"), "got: {s}");
    }

    #[test]
    fn human_renders_string_without_quotes() {
        let s = human(&serde_json::json!("hello"));
        assert_eq!(s, "hello");
    }

    #[test]
    fn exit_push_notification_payload_is_well_formed() {
        // Mirror the construction inside `spawn_exit_push` without a transport:
        // a finished SessionInfo -> CustomNotification on CHANNEL_METHOD.
        let info = crate::types::SessionInfo {
            id: "pty_abcd".to_string(),
            title: "t".to_string(),
            command: "npm".to_string(),
            args: vec!["run".to_string(), "dev".to_string()],
            workdir: ".".to_string(),
            status: crate::types::Status::Exited,
            pid: Some(123),
            exit_code: Some(0),
            line_count: 4,
            timed_out: false,
        };
        let params = crate::channel::exit_params_for(&info);
        let notification = CustomNotification::new(crate::channel::CHANNEL_METHOD, Some(params));

        assert_eq!(notification.method, "notifications/claude/channel");
        let p = notification.params.as_ref().expect("params present");
        assert_eq!(p["meta"]["session_id"], "pty_abcd");
        assert_eq!(p["meta"]["exit_code"], "0");
        assert!(p["content"].as_str().unwrap().contains("npm run dev"));

        // It wraps into the ServerNotification enum without loss.
        let server_notification = ServerNotification::CustomNotification(notification);
        match server_notification {
            ServerNotification::CustomNotification(n) => {
                assert_eq!(n.method, crate::channel::CHANNEL_METHOD);
            }
            _ => panic!("expected CustomNotification variant"),
        }
    }

    #[test]
    fn human_renders_array_as_lines() {
        let s = human(&serde_json::json!([
            {"id": "a", "status": "running"},
            {"id": "b", "status": "dead"}
        ]));
        assert!(s.contains("id: a"), "got: {s}");
        assert!(s.contains("id: b"), "got: {s}");
    }

    #[test]
    fn advertises_claude_channel_capability() {
        use rmcp::ServerHandler;
        let policy = std::sync::Arc::new(crate::policy::PolicyEngine::from_lists(
            vec![],
            vec![],
            true,
        ));
        let srv = VoxServer::new(crate::pty::manager::PtyManager::new(), policy);
        let info = srv.get_info();
        let experimental = info
            .capabilities
            .experimental
            .expect("experimental capabilities must be advertised");
        assert!(
            experimental.contains_key("claude/channel"),
            "server must declare the claude/channel capability or Claude Code drops channel pushes"
        );
    }
}
