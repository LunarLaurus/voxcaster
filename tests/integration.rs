use voxcaster::pty::manager::PtyManager;
use voxcaster::types::{SpawnOptions, Status};

fn echo(line: &str) -> SpawnOptions {
    #[cfg(windows)]
    let (command, args) = ("cmd".to_string(), vec!["/C".into(), format!("echo {line}")]);
    #[cfg(not(windows))]
    let (command, args) = ("/bin/echo".to_string(), vec![line.to_string()]);
    SpawnOptions {
        command,
        args,
        workdir: None,
        env: vec![],
        title: None,
        notify_on_exit: false,
        timeout_seconds: None,
    }
}

#[tokio::test]
async fn full_lifecycle() {
    let m = PtyManager::new();
    let id = m.spawn(echo("integration-ok")).unwrap();
    let info = m.wait(&id, Some(10)).await.unwrap();
    assert_eq!(info.status, Status::Exited);
    assert_eq!(info.exit_code, Some(0));
    let slice = m.read(&id, 0, None, false).unwrap();
    assert!(
        slice.lines.iter().any(|l| l.contains("integration-ok")),
        "got {:?}",
        slice.lines
    );
}
