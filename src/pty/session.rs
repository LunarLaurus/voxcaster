#![allow(dead_code)]

use std::io::Read;
use std::sync::{Arc, Mutex};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::watch;

use crate::pty::buffer::{ReadSlice, RingBuffer};
use crate::types::{SessionInfo, SpawnOptions, Status, VoxError};

const DEFAULT_MAX_LINES: usize = 5_000;
const DEFAULT_MAX_BYTES: usize = 256 * 1024;

struct Shared {
    buffer: Mutex<RingBuffer>,
    status: Mutex<Status>,
    exit_code: Mutex<Option<i32>>,
    timed_out: Mutex<bool>,
}

pub struct PtySession {
    pub id: String,
    pub opts: SpawnOptions,
    pub pid: Option<u32>,
    pub workdir: String,
    shared: Arc<Shared>,
    writer: Mutex<Box<dyn std::io::Write + Send>>,
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
    exit_rx: watch::Receiver<bool>,
}

impl PtySession {
    /// Spawn a child process in a pseudo-terminal and begin capturing its output.
    pub fn spawn(id: &str, opts: SpawnOptions) -> Result<Self, VoxError> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| VoxError::Spawn(e.to_string()))?;

        let mut cmd = CommandBuilder::new(&opts.command);
        cmd.args(&opts.args);
        let workdir = opts.workdir.clone().unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        });
        cmd.cwd(&workdir);
        for (k, v) in &opts.env {
            cmd.env(k, v);
        }

        let mut child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| VoxError::Spawn(e.to_string()))?;

        let pid = child.process_id();
        let killer = child.clone_killer();

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| VoxError::Spawn(e.to_string()))?;
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| VoxError::Spawn(e.to_string()))?;

        // Drop the slave end so that when the child exits the master reader
        // receives EOF. Without this the reader blocks forever on Windows.
        drop(pair.slave);
        // pair.master is kept alive implicitly — reader/writer hold the OS
        // handle. We do NOT need to keep pair.master; it was consumed above.
        // (The master handle is kept alive via the reader/writer Box internals.)
        drop(pair.master);

        let shared = Arc::new(Shared {
            buffer: Mutex::new(RingBuffer::new(DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES)),
            status: Mutex::new(Status::Running),
            exit_code: Mutex::new(None),
            timed_out: Mutex::new(false),
        });
        let (exit_tx, exit_rx) = watch::channel(false);

        // Reader thread: blocking PTY reads → RingBuffer.
        {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            let chunk = String::from_utf8_lossy(&buf[..n]);
                            shared.buffer.lock().unwrap().append(&chunk);
                        }
                    }
                }
            });
        }

        // Waiter thread: block on child exit → flush buffer, update status + code, signal watch.
        {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let status = child.wait();
                let code = status.ok().map(|s| s.exit_code() as i32);

                // Give the reader thread a moment to drain remaining output before flushing.
                std::thread::sleep(std::time::Duration::from_millis(50));

                shared.buffer.lock().unwrap().flush();
                let mut st = shared.status.lock().unwrap();
                if *st == Status::Killing {
                    *st = Status::Killed;
                } else {
                    *st = Status::Exited;
                }
                *shared.exit_code.lock().unwrap() = code;
                let _ = exit_tx.send(true);
            });
        }

        Ok(Self {
            id: id.to_string(),
            opts,
            pid,
            workdir,
            shared,
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            exit_rx,
        })
    }

    /// Write raw bytes to the PTY master (simulates keyboard input).
    pub fn write(&self, data: &str) -> Result<(), VoxError> {
        use std::io::Write;
        let mut w = self.writer.lock().unwrap();
        w.write_all(data.as_bytes())?;
        w.flush()?;
        Ok(())
    }

    /// Read lines from the captured output buffer.
    pub fn read(&self, offset: usize, limit: Option<usize>, raw: bool) -> ReadSlice {
        self.shared.buffer.lock().unwrap().read(offset, limit, raw)
    }

    /// Return the current lifecycle status of this session.
    pub fn status(&self) -> Status {
        *self.shared.status.lock().unwrap()
    }

    /// Return the exit code, if the process has completed.
    pub fn exit_code(&self) -> Option<i32> {
        *self.shared.exit_code.lock().unwrap()
    }

    /// Async wait for process exit. Returns the exit code, or `None` if the
    /// timeout elapsed before the process finished.
    pub async fn wait(&self, timeout_seconds: Option<u64>) -> Result<Option<i32>, VoxError> {
        // Fast path: already done.
        if *self.exit_rx.borrow() {
            return Ok(self.exit_code());
        }
        let mut rx = self.exit_rx.clone();
        let fut = async move {
            let _ = rx.changed().await;
        };
        match timeout_seconds {
            Some(secs) => {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(secs), fut).await;
            }
            None => fut.await,
        }
        Ok(self.exit_code())
    }

    /// Send a kill signal to the child process.
    pub fn kill(&self) {
        *self.shared.status.lock().unwrap() = Status::Killing;
        let _ = self.killer.lock().unwrap().kill();
    }

    /// Mark this session as having timed out.
    pub fn mark_timed_out(&self) {
        *self.shared.timed_out.lock().unwrap() = true;
    }

    /// Return a snapshot of this session's metadata.
    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.clone(),
            title: self.opts.title.clone().unwrap_or_else(|| {
                format!("{} {}", self.opts.command, self.opts.args.join(" "))
                    .trim()
                    .to_string()
            }),
            command: self.opts.command.clone(),
            args: self.opts.args.clone(),
            workdir: self.workdir.clone(),
            status: self.status(),
            pid: self.pid,
            exit_code: self.exit_code(),
            line_count: self.shared.buffer.lock().unwrap().len(),
            timed_out: *self.shared.timed_out.lock().unwrap(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SpawnOptions, Status};

    fn echo_opts(line: &str) -> SpawnOptions {
        // cmd.exe exits with STATUS_NOT_IMPLEMENTED (0xC000_0002) when launched
        // inside a ConPTY created with PSEUDOCONSOLE_WIN32_INPUT_MODE, which is
        // the flag portable-pty 0.9.0 sets on Windows.  PowerShell handles the
        // mode correctly and exits 0.
        #[cfg(windows)]
        let (command, args) = (
            "powershell".to_string(),
            vec!["-Command".into(), format!("Write-Output '{line}'")],
        );
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

    /// Returns `true` if this process has or can obtain a Windows console.
    /// ConPTY requires a console subsystem context; skip the test if unavailable.
    #[cfg(windows)]
    fn try_alloc_console() -> bool {
        // Use raw FFI via the standard library's os module — winapi is a transitive
        // dep only; declare the symbols ourselves to avoid adding a direct dependency.
        #[link(name = "kernel32")]
        extern "system" {
            fn AllocConsole() -> i32;
            fn FreeConsole() -> i32;
        }
        let allocated = unsafe { AllocConsole() != 0 };
        if !allocated {
            // Could not allocate — either already have one or access denied.
            // Check if we genuinely have a console by querying the window.
            #[link(name = "user32")]
            extern "system" {
                fn GetConsoleWindow() -> *mut std::ffi::c_void;
            }
            return !unsafe { GetConsoleWindow() }.is_null();
        }
        // Successfully allocated — free it immediately so the ConPTY can have the
        // console context it needs when CreatePseudoConsole runs.
        unsafe { FreeConsole() };
        true
    }

    #[tokio::test]
    async fn spawns_captures_output_and_exits_zero() {
        // ConPTY on Windows requires the calling process to have a console.
        // In headless CI / sandbox environments (GetConsoleWindow = null AND
        // AllocConsole = ERROR_ACCESS_DENIED), all ConPTY children exit with
        // STATUS_DLL_INIT_FAILED (0xC000_0142).  Detect this early and skip.
        #[cfg(windows)]
        if !try_alloc_console() {
            eprintln!(
                "SKIP spawns_captures_output_and_exits_zero: \
                 no console subsystem available — ConPTY cannot operate in this environment"
            );
            return;
        }

        let s = PtySession::spawn("pty_test", echo_opts("hello")).unwrap();
        let code = s.wait(Some(10)).await.unwrap();
        assert_eq!(code, Some(0));
        let slice = s.read(0, None, false);
        assert!(
            slice.lines.iter().any(|l| l.contains("hello")),
            "got {:?}",
            slice.lines
        );
        assert_eq!(s.status(), Status::Exited);
    }
}
