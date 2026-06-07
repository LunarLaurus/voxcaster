mod channel;
mod policy;
mod pty;
mod server;
mod types;

use std::path::Path;
use std::sync::Arc;

use rmcp::ServiceExt;

use policy::PolicyEngine;
use pty::manager::PtyManager;
use server::VoxServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Resolve the policy file path; warn on stderr if it is absent so an operator
    // is never silently running with a wide-open (allow-all) policy.
    let policy_path =
        std::env::var("VOXCASTER_POLICY").unwrap_or_else(|_| "voxcaster-policy.toml".to_string());
    if !Path::new(&policy_path).exists() {
        eprintln!(
            "voxcaster: warning: policy file '{policy_path}' not found; \
             running with the permissive default (allow-by-default, no deny rules). \
             Set VOXCASTER_POLICY or create the file to enforce a command policy."
        );
    }
    let policy = Arc::new(PolicyEngine::from_file(&policy_path));
    let manager = PtyManager::new();
    let server = VoxServer::new(manager.clone(), policy);

    // Reap all PTYs if we receive Ctrl-C / termination.
    let reaper = manager.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            reaper.reap_all();
            std::process::exit(0);
        }
    });

    // Serve the MCP protocol over stdio until the client disconnects (stdin EOF).
    // rmcp::transport::stdio() returns (tokio::io::Stdin, tokio::io::Stdout) which
    // implements IntoTransport via the async-rw adapter.
    let service = server.serve(rmcp::transport::stdio()).await?;
    service.waiting().await.ok();

    // Client gone: reap any surviving PTYs.
    manager.reap_all();
    Ok(())
}
