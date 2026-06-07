/// Integration test: proves the voxcaster MCP server emits a
/// `notifications/claude/channel` notification when a `notify_on_exit` PTY
/// session exits.
///
/// Strategy A — rmcp client + child-process transport.
/// `ClientHandler::on_custom_notification` is dispatched for
/// `ServerNotification::CustomNotification`, which is how voxcaster sends the
/// channel exit-push.  The handler captures the notification into an
/// `Arc<Mutex<Vec<CustomNotification>>>` shared with the test body.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rmcp::model::{CallToolRequestParams, CustomNotification};
use rmcp::service::NotificationContext;
use rmcp::transport::TokioChildProcess;
use rmcp::{serve_client, ClientHandler, RoleClient};
use serde_json::Value;
use tokio::process::Command;
use tokio::time::timeout;

// ---------------------------------------------------------------------------
// Test handler
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct ChannelCollector {
    received: Arc<Mutex<Vec<CustomNotification>>>,
}

impl ChannelCollector {
    fn new() -> Self {
        Self {
            received: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn received(&self) -> Arc<Mutex<Vec<CustomNotification>>> {
        self.received.clone()
    }
}

impl ClientHandler for ChannelCollector {
    fn on_custom_notification(
        &self,
        notification: CustomNotification,
        _context: NotificationContext<RoleClient>,
    ) -> impl std::future::Future<Output = ()> + Send + '_ {
        let received = self.received.clone();
        async move {
            println!(
                "[channel_push] custom notification: method={} params={:?}",
                notification.method, notification.params
            );
            received.lock().unwrap().push(notification);
        }
    }
}

// ---------------------------------------------------------------------------
// Test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn channel_exit_push_emits_notification() {
    // Locate the voxcaster binary built by cargo.
    let bin = env!("CARGO_BIN_EXE_voxcaster");
    println!("[channel_push] binary: {bin}");

    // Write a temporary permissive policy file so the server does not block the
    // echo command.  We point VOXCASTER_POLICY at this file via the env var.
    let policy_dir = std::env::temp_dir();
    let policy_path = policy_dir.join("voxcaster-channel-test-policy.toml");
    std::fs::write(
        &policy_path,
        "allow_by_default = true\nallow = []\ndeny  = []\n",
    )
    .expect("failed to write temp policy file");

    // Spawn voxcaster as a child MCP server process.
    let mut cmd = Command::new(bin);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::inherit())
        .env("VOXCASTER_POLICY", &policy_path);

    let transport = TokioChildProcess::new(cmd).expect("failed to spawn voxcaster");

    // Build the handler and keep a handle to its notification store.
    let handler = ChannelCollector::new();
    let store = handler.received();

    // Perform the MCP initialize handshake and start the client loop.
    let client = timeout(Duration::from_secs(15), serve_client(handler, transport))
        .await
        .expect("client initialization timed out")
        .expect("MCP initialize handshake failed");

    let peer = client.peer();

    // Build the pty_spawn arguments.
    #[cfg(windows)]
    let (cmd_name, cmd_args) = ("cmd", vec!["/C", "echo channel-test-ok"]);
    #[cfg(not(windows))]
    let (cmd_name, cmd_args) = ("/bin/echo", vec!["channel-test-ok"]);

    let mut arguments = serde_json::Map::new();
    arguments.insert("command".into(), Value::String(cmd_name.into()));
    arguments.insert(
        "args".into(),
        Value::Array(
            cmd_args
                .iter()
                .map(|s| Value::String(s.to_string()))
                .collect(),
        ),
    );
    arguments.insert(
        "description".into(),
        Value::String("channel exit-push integration test".into()),
    );
    arguments.insert("notify_on_exit".into(), Value::Bool(true));

    let params = CallToolRequestParams::new("pty_spawn").with_arguments(arguments);

    println!("[channel_push] calling pty_spawn …");
    let tool_result = timeout(Duration::from_secs(15), peer.call_tool(params))
        .await
        .expect("pty_spawn call timed out")
        .expect("pty_spawn tool call failed");

    println!("[channel_push] pty_spawn result: {tool_result:?}");

    // Poll for the channel notification (up to 20 seconds; cmd /C echo exits quickly).
    let notification = timeout(Duration::from_secs(20), async {
        loop {
            {
                let guard = store.lock().unwrap();
                if let Some(n) = guard
                    .iter()
                    .find(|n| n.method == "notifications/claude/channel")
                    .cloned()
                {
                    return n;
                }
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("timed out waiting for notifications/claude/channel notification");

    println!(
        "[channel_push] received notification: method={} params={:?}",
        notification.method, notification.params
    );

    // Assertions.
    assert_eq!(
        notification.method, "notifications/claude/channel",
        "notification method mismatch"
    );

    let params = notification
        .params
        .as_ref()
        .expect("notification params must be present");

    let meta = params
        .get("meta")
        .expect("params must contain a 'meta' key");

    let session_id = meta
        .get("session_id")
        .and_then(Value::as_str)
        .expect("meta.session_id must be a string");
    assert!(
        session_id.starts_with("pty_"),
        "session_id '{session_id}' should start with 'pty_'"
    );

    let exit_code = meta
        .get("exit_code")
        .and_then(Value::as_str)
        .expect("meta.exit_code must be a string");
    assert_eq!(exit_code, "0", "exit_code should be '0'");

    println!("[channel_push] PASS — session_id={session_id} exit_code={exit_code}");

    // Cancel the client; the child process will be killed by TokioChildProcess's Drop impl.
    client.cancellation_token().cancel();

    // Remove the temporary policy file.
    let _ = std::fs::remove_file(&policy_path);
}
