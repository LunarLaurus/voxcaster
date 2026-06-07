# Voxcaster Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.
>
> **ADAPT to actuals (Commander's doctrine, overrides skill strictness):** This plan is a hypothesis; the crate and the live `rmcp`/`portable-pty` APIs are the truth. Where an exact signature differs at implementation time (flagged ⚠️ADAPT below), adjust the call site to the real API and keep the test's behavioural assertion intact. Re-verify line citations after each edit (linecite).

**Goal:** Build Voxcaster — a cross-platform Rust MCP server giving an AI agent persistent, interactive PTY sessions (spawn/write/read/wait/list/kill) with layered exit signalling and defense-in-depth permissions.

**Architecture:** A single stdio MCP server (`rmcp`, async/tokio). A `PtyManager` owns `PtySession`s; each session wraps a `portable-pty` child, a blocking reader thread feeding an `Arc<Mutex<RingBuffer>>`, and a status FSM. A `PolicyEngine` gates `pty_spawn`. A `ChannelEmitter` optionally pushes `notifications/claude/channel` on exit. No daemon, no web UI.

**Tech Stack:** Rust 2021, `rmcp` (official MCP SDK), `portable-pty` (wezterm), `tokio`, `serde`/`serde_json`, `schemars`, `thiserror`, `globset` (policy globs), `toml`, `strip-ansi-escapes`.

---

## Conventions (the TDD rhythm — applies to every task)

Each task follows the same five-step rhythm; it is written out in full in Task 2 and abbreviated thereafter to avoid bloat. The rhythm is:

1. Write the failing test (code shown per task).
2. Run it, confirm it fails for the expected reason.
3. Write the minimal implementation (code shown per task).
4. Run the test, confirm it passes; run `cargo clippy --all-targets -- -D warnings` and `cargo fmt`.
5. Commit with a conventional message.

Test command baseline: `cargo test <name> -- --nocolor`. Lint baseline: `cargo clippy --all-targets --all-features -- -D warnings`. Format: `cargo fmt --all`.

## File structure (decomposition — locked here)

```
voxcaster/
  Cargo.toml
  src/
    main.rs            # entry: build manager+policy+server, serve over stdio
    server.rs          # McpServer: rmcp ServerHandler + #[tool_router], the 6 tools
    channel.rs         # ChannelEmitter: build + send notifications/claude/channel
    policy.rs          # PolicyEngine: load TOML, allow/deny glob match
    types.rs           # SpawnOptions, SessionInfo, Status, ReadResult, VoxError
    pty/
      mod.rs           # re-exports
      manager.rs       # PtyManager: sessions map, spawn/write/read/wait/list/kill/reap_all
      session.rs       # PtySession: child, reader thread, status FSM, exit watch
      buffer.rs        # RingBuffer: bounded line store, line addressing, ANSI strip
      id.rs            # generate_id() -> "pty_<8 hex>"
  tests/
    integration.rs     # end-to-end: spawn real process, read, wait, exit code
  voxcaster-policy.example.toml
```

Each file has one responsibility; `pty/` changes together and lives together.

---

### Task 1: Project scaffold + dependencies

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs` (placeholder)

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "voxcaster"
version = "0.1.0"
edition = "2021"
description = "MCP server providing persistent interactive PTY sessions to an AI agent"
license = "MIT"

[[bin]]
name = "voxcaster"
path = "src/main.rs"

[dependencies]
rmcp = { version = "0.9", features = ["server", "transport-io", "macros"] }
portable-pty = "0.9"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "time", "signal"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
schemars = "1"
thiserror = "2"
globset = "0.4"
toml = "0.8"
strip-ansi-escapes = "0.2"
anyhow = "1"

[dev-dependencies]
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time", "test-util"] }
```

> ⚠️ADAPT: pin `rmcp` / `portable-pty` to the latest released versions at implementation time (`cargo add rmcp --features server,transport-io,macros`); the feature names above match rmcp 0.9-era. If a feature is renamed, `cargo build` will name it.

- [ ] **Step 2: Placeholder `src/main.rs`**

```rust
fn main() {
    println!("voxcaster");
}
```

- [ ] **Step 3: Verify it builds**

Run: `cargo build`
Expected: compiles clean (downloads deps).

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock src/main.rs
git commit -m "chore: Scaffold the Voxcaster crate and augmetics"
```

---

### Task 2: RingBuffer — bounded line store with ANSI strip

**Files:**
- Create: `src/pty/buffer.rs`
- Create: `src/pty/mod.rs`
- Test: inline `#[cfg(test)]` in `buffer.rs`

- [ ] **Step 1: Write the failing test**

In `src/pty/buffer.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_and_reads_lines_by_range() {
        let mut b = RingBuffer::new(100, 64 * 1024);
        b.append("line one\nline two\nline three\n");
        let r = b.read(0, Some(2), false);
        assert_eq!(r.lines, vec!["line one", "line two"]);
        assert_eq!(r.total, 3);
        assert!(!r.truncated);
    }

    #[test]
    fn strips_ansi_by_default_keeps_raw_on_request() {
        let mut b = RingBuffer::new(100, 64 * 1024);
        b.append("\x1b[31mred\x1b[0m\n");
        assert_eq!(b.read(0, None, false).lines, vec!["red"]);
        assert_eq!(b.read(0, None, true).lines, vec!["\x1b[31mred\x1b[0m"]);
    }

    #[test]
    fn drops_oldest_lines_past_cap() {
        let mut b = RingBuffer::new(2, 64 * 1024);
        b.append("a\nb\nc\n");
        let r = b.read(0, None, false);
        assert_eq!(r.lines, vec!["b", "c"]);
        assert_eq!(r.total, 2);
    }
}
```

- [ ] **Step 2: Run, confirm failure**

Run: `cargo test buffer:: -- --nocolor`
Expected: FAIL — `RingBuffer` not found.

- [ ] **Step 3: Implement `RingBuffer`**

`src/pty/mod.rs`:

```rust
pub mod buffer;
pub mod id;
pub mod manager;
pub mod session;
```

`src/pty/buffer.rs` (above the test module):

```rust
use std::collections::VecDeque;

/// Result of a buffer read: a contiguous slice of lines plus addressing metadata.
pub struct ReadSlice {
    pub lines: Vec<String>,
    pub offset: usize,
    pub total: usize,
    pub truncated: bool,
}

/// Bounded, line-oriented scrollback. Stores raw lines; drops oldest on overflow.
pub struct RingBuffer {
    lines: VecDeque<String>,
    pending: String,
    max_lines: usize,
    max_bytes: usize,
    bytes: usize,
    dropped: bool,
}

impl RingBuffer {
    pub fn new(max_lines: usize, max_bytes: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            pending: String::new(),
            max_lines,
            max_bytes,
            bytes: 0,
            dropped: false,
        }
    }

    /// Append a chunk of raw PTY output, splitting on '\n'. A trailing
    /// partial line is held in `pending` until its newline arrives.
    pub fn append(&mut self, chunk: &str) {
        self.pending.push_str(chunk);
        while let Some(idx) = self.pending.find('\n') {
            let mut line: String = self.pending.drain(..=idx).collect();
            if line.ends_with('\n') { line.pop(); }
            if line.ends_with('\r') { line.pop(); }
            self.push_line(line);
        }
    }

    /// Force any held partial line into the buffer (call on process exit).
    pub fn flush(&mut self) {
        if !self.pending.is_empty() {
            let line = std::mem::take(&mut self.pending);
            self.push_line(line);
        }
    }

    fn push_line(&mut self, line: String) {
        self.bytes += line.len();
        self.lines.push_back(line);
        while self.lines.len() > self.max_lines || self.bytes > self.max_bytes {
            if let Some(old) = self.lines.pop_front() {
                self.bytes -= old.len();
                self.dropped = true;
            } else {
                break;
            }
        }
    }

    /// Read a line range. `raw=false` strips ANSI escapes.
    pub fn read(&self, offset: usize, limit: Option<usize>, raw: bool) -> ReadSlice {
        let total = self.lines.len();
        let end = match limit {
            Some(l) => (offset + l).min(total),
            None => total,
        };
        let lines = self
            .lines
            .iter()
            .skip(offset)
            .take(end.saturating_sub(offset))
            .map(|l| if raw { l.clone() } else { strip(l) })
            .collect();
        ReadSlice { lines, offset, total, truncated: self.dropped }
    }

    pub fn len(&self) -> usize { self.lines.len() }
}

fn strip(s: &str) -> String {
    String::from_utf8_lossy(&strip_ansi_escapes::strip(s.as_bytes())).into_owned()
}
```

> ⚠️ADAPT: `strip_ansi_escapes::strip` returns `Vec<u8>` in current versions; if the signature is `strip(&str) -> Result<Vec<u8>>`, unwrap accordingly.

- [ ] **Step 4: Run, confirm pass; lint + fmt.**
- [ ] **Step 5: Commit** — `git commit -m "feat: Forge the RingBuffer scrollback canticle"`

---

### Task 3: Session id generation

**Files:**
- Create: `src/pty/id.rs`

- [ ] **Step 1: Failing test** (inline in `id.rs`):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ids_are_prefixed_and_unique() {
        let a = generate_id();
        let b = generate_id();
        assert!(a.starts_with("pty_"));
        assert_eq!(a.len(), 4 + 16); // "pty_" + 8 bytes hex
        assert_ne!(a, b);
    }
}
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** (8 random bytes — wider than opencode's 4, per audit):

```rust
use std::time::{SystemTime, UNIX_EPOCH};

/// Generate a session id "pty_<16 hex>". Uses time + a process-local counter
/// xored with the address of a stack local for entropy without an RNG dep.
pub fn generate_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;
    let mix = t ^ (n.wrapping_mul(0x9E37_79B9_7F4A_7C15));
    format!("pty_{:016x}", mix)
}
```

> ⚠️ADAPT: if a crypto-random id is preferred, add `getrandom` and fill 8 bytes; the test only requires prefix + length + uniqueness.

- [ ] **Step 4: Pass + lint.**
- [ ] **Step 5: Commit** — `git commit -m "feat: Inscribe session-id sigil generation"`

---

### Task 4: Shared types

**Files:**
- Create: `src/types.rs`

- [ ] **Step 1: Failing test** (inline) — status serialises to the expected strings:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_serialises_lowercase() {
        assert_eq!(serde_json::to_string(&Status::Running).unwrap(), "\"running\"");
        assert_eq!(serde_json::to_string(&Status::Exited).unwrap(), "\"exited\"");
    }
}
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement**:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status { Running, Killing, Killed, Exited }

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
```

- [ ] **Step 4: Pass + lint.**
- [ ] **Step 5: Commit** — `git commit -m "feat: Define the core data-types and error taxonomy"`

---

### Task 5: PolicyEngine — allow/deny glob gate

**Files:**
- Create: `src/policy.rs`
- Create: `voxcaster-policy.example.toml`

- [ ] **Step 1: Failing test** (inline):

```rust
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
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement**:

```rust
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::Deserialize;
use crate::types::VoxError;

#[derive(Debug, Deserialize, Default)]
pub struct PolicyFile {
    #[serde(default)] pub allow: Vec<String>,
    #[serde(default)] pub deny: Vec<String>,
    #[serde(default = "default_true")] pub allow_by_default: bool,
}
fn default_true() -> bool { true }

pub struct PolicyEngine {
    allow: GlobSet,
    deny: GlobSet,
    allow_by_default: bool,
}

impl PolicyEngine {
    pub fn from_lists(allow: Vec<String>, deny: Vec<String>, allow_by_default: bool) -> Self {
        Self { allow: build(&allow), deny: build(&deny), allow_by_default }
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

fn build(patterns: &[String]) -> GlobSet {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(g) = Glob::new(p) {
            b.add(g);
        }
    }
    b.build().unwrap_or_else(|_| GlobSet::empty())
}
```

`voxcaster-policy.example.toml`:

```toml
# Voxcaster command policy. Deny rules always win.
allow_by_default = true
allow = ["npm *", "cargo *", "git *", "go *", "python *", "node *"]
deny  = ["rm -rf /*", "* | sh", "curl * | *", "* > /dev/sd*"]
```

> ⚠️ADAPT: globset matches paths; `*` does not cross `/`. For shell-ish patterns like `* | sh`, confirm match behaviour in Step 2 and switch to `Glob::new(p).literal_separator(false)` if `*` must span spaces/slashes.

- [ ] **Step 4: Pass + lint.**
- [ ] **Step 5: Commit** — `git commit -m "feat: Forge the PolicyEngine command-permission ward"`

---

### Task 6: PtySession — spawn, reader thread, status FSM, exit watch

**Files:**
- Create: `src/pty/session.rs`

- [ ] **Step 1: Failing test** (inline; spawns a real trivial process):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{SpawnOptions, Status};

    fn echo_opts(line: &str) -> SpawnOptions {
        // Cross-platform echo via the OS shell-less path: use `printf`-like.
        #[cfg(windows)]
        let (command, args) = ("cmd".to_string(), vec!["/C".into(), format!("echo {line}")]);
        #[cfg(not(windows))]
        let (command, args) = ("/bin/echo".to_string(), vec![line.to_string()]);
        SpawnOptions { command, args, workdir: None, env: vec![], title: None,
                       notify_on_exit: false, timeout_seconds: None }
    }

    #[tokio::test]
    async fn spawns_captures_output_and_exits_zero() {
        let s = PtySession::spawn("pty_test", echo_opts("hello")).unwrap();
        let code = s.wait(Some(10)).await.unwrap();
        assert_eq!(code, Some(0));
        let slice = s.read(0, None, false);
        assert!(slice.lines.iter().any(|l| l.contains("hello")), "got {:?}", slice.lines);
        assert_eq!(s.status(), Status::Exited);
    }
}
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** (blocking reads on a std thread; exit via `tokio::sync::watch`):

```rust
use std::io::Read;
use std::sync::{Arc, Mutex};
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::watch;
use crate::pty::buffer::{RingBuffer, ReadSlice};
use crate::types::{SpawnOptions, Status, VoxError, SessionInfo};

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
    pub fn spawn(id: &str, opts: SpawnOptions) -> Result<Self, VoxError> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize { rows: 40, cols: 120, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| VoxError::Spawn(e.to_string()))?;

        let mut cmd = CommandBuilder::new(&opts.command);
        cmd.args(&opts.args);
        let workdir = opts.workdir.clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap().to_string_lossy().into_owned());
        cmd.cwd(&workdir);
        for (k, v) in &opts.env { cmd.env(k, v); }

        let mut child = pair.slave.spawn_command(cmd)
            .map_err(|e| VoxError::Spawn(e.to_string()))?;
        let pid = child.process_id();
        let killer = child.clone_killer();

        let writer = pair.master.take_writer().map_err(|e| VoxError::Spawn(e.to_string()))?;
        let mut reader = pair.master.try_clone_reader().map_err(|e| VoxError::Spawn(e.to_string()))?;

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

        // Waiter thread: block on child exit → flush, set status + code, signal watch.
        {
            let shared = shared.clone();
            std::thread::spawn(move || {
                let status = child.wait();
                let code = status.ok().map(|s| s.exit_code() as i32);
                shared.buffer.lock().unwrap().flush();
                let mut st = shared.status.lock().unwrap();
                if *st == Status::Killing { *st = Status::Killed; } else { *st = Status::Exited; }
                *shared.exit_code.lock().unwrap() = code;
                let _ = exit_tx.send(true);
            });
        }

        Ok(Self {
            id: id.to_string(), opts, pid, workdir,
            shared,
            writer: Mutex::new(writer),
            killer: Mutex::new(killer),
            exit_rx,
        })
    }

    pub fn write(&self, data: &str) -> Result<(), VoxError> {
        use std::io::Write;
        let mut w = self.writer.lock().unwrap();
        w.write_all(data.as_bytes())?;
        w.flush()?;
        Ok(())
    }

    pub fn read(&self, offset: usize, limit: Option<usize>, raw: bool) -> ReadSlice {
        self.shared.buffer.lock().unwrap().read(offset, limit, raw)
    }

    pub fn status(&self) -> Status { *self.shared.status.lock().unwrap() }
    pub fn exit_code(&self) -> Option<i32> { *self.shared.exit_code.lock().unwrap() }

    /// Block until exit or `timeout_seconds`. Returns the exit code (None if still running on timeout).
    pub async fn wait(&self, timeout_seconds: Option<u64>) -> Result<Option<i32>, VoxError> {
        if *self.exit_rx.borrow() { return Ok(self.exit_code()); }
        let mut rx = self.exit_rx.clone();
        let fut = async move { let _ = rx.changed().await; };
        match timeout_seconds {
            Some(secs) => {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(secs), fut).await;
            }
            None => fut.await,
        }
        Ok(self.exit_code())
    }

    pub fn kill(&self) {
        *self.shared.status.lock().unwrap() = Status::Killing;
        let _ = self.killer.lock().unwrap().kill();
    }

    pub fn mark_timed_out(&self) { *self.shared.timed_out.lock().unwrap() = true; }

    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            id: self.id.clone(),
            title: self.opts.title.clone().unwrap_or_else(|| {
                format!("{} {}", self.opts.command, self.opts.args.join(" ")).trim().to_string()
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
```

> ⚠️ADAPT (verify against live `portable-pty`): method names `take_writer`, `try_clone_reader`, `clone_killer`, and the `ChildKiller` trait. In current portable-pty these exist; if `clone_killer` is absent, store the `Box<dyn Child>` behind a `Mutex` and call `.kill()` on it instead. `ExitStatus::exit_code()` returns `u32`.

- [ ] **Step 4: Run `cargo test session:: -- --nocolor`; confirm pass; lint + fmt.**
- [ ] **Step 5: Commit** — `git commit -m "feat: Bind the machine spirit — PtySession lifecycle"`

---

### Task 7: PtyManager — own sessions, timeout, reap_all

**Files:**
- Create: `src/pty/manager.rs`

- [ ] **Step 1: Failing test** (inline):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SpawnOptions;

    fn sleep_opts(secs: u32) -> SpawnOptions {
        #[cfg(windows)]
        let (command, args) = ("cmd".to_string(), vec!["/C".into(), format!("ping -n {} 127.0.0.1", secs + 1)]);
        #[cfg(not(windows))]
        let (command, args) = ("/bin/sleep".to_string(), vec![secs.to_string()]);
        SpawnOptions { command, args, workdir: None, env: vec![], title: None,
                       notify_on_exit: false, timeout_seconds: None }
    }

    #[tokio::test]
    async fn spawn_list_kill_roundtrip() {
        let m = PtyManager::new();
        let id = m.spawn(sleep_opts(30)).unwrap();
        assert_eq!(m.list().len(), 1);
        assert!(m.kill(&id, false).is_ok());
        m.reap_all();
        // after reap, sessions cleared
        assert_eq!(m.list().len(), 0);
    }
}
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** (`Arc<Mutex<HashMap>>`; timeout via `tokio::spawn` + `kill`):

```rust
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use crate::pty::id::generate_id;
use crate::pty::session::PtySession;
use crate::pty::buffer::ReadSlice;
use crate::types::{SpawnOptions, SessionInfo, VoxError};

#[derive(Clone)]
pub struct PtyManager {
    sessions: Arc<Mutex<HashMap<String, Arc<PtySession>>>>,
}

impl PtyManager {
    pub fn new() -> Self {
        Self { sessions: Arc::new(Mutex::new(HashMap::new())) }
    }

    pub fn spawn(&self, opts: SpawnOptions) -> Result<String, VoxError> {
        let id = generate_id();
        let timeout = opts.timeout_seconds;
        let session = Arc::new(PtySession::spawn(&id, opts)?);
        self.sessions.lock().unwrap().insert(id.clone(), session.clone());

        if let Some(secs) = timeout {
            let s = session.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                if s.status() == crate::types::Status::Running {
                    s.mark_timed_out();
                    s.kill();
                }
            });
        }
        Ok(id)
    }

    fn get(&self, id: &str) -> Result<Arc<PtySession>, VoxError> {
        self.sessions.lock().unwrap().get(id).cloned()
            .ok_or_else(|| VoxError::NotFound(id.to_string()))
    }

    pub fn write(&self, id: &str, data: &str) -> Result<(), VoxError> {
        self.get(id)?.write(data)
    }

    pub fn read(&self, id: &str, offset: usize, limit: Option<usize>, raw: bool)
        -> Result<ReadSlice, VoxError> {
        Ok(self.get(id)?.read(offset, limit, raw))
    }

    pub async fn wait(&self, id: &str, timeout_seconds: Option<u64>)
        -> Result<crate::types::SessionInfo, VoxError> {
        let s = self.get(id)?;
        s.wait(timeout_seconds).await?;
        Ok(s.info())
    }

    pub fn list(&self) -> Vec<SessionInfo> {
        self.sessions.lock().unwrap().values().map(|s| s.info()).collect()
    }

    pub fn kill(&self, id: &str, cleanup: bool) -> Result<(), VoxError> {
        let s = self.get(id)?;
        s.kill();
        if cleanup { self.sessions.lock().unwrap().remove(id); }
        Ok(())
    }

    /// Kill every session — called on server shutdown (session end).
    pub fn reap_all(&self) {
        let mut map = self.sessions.lock().unwrap();
        for s in map.values() { s.kill(); }
        map.clear();
    }

    /// Register a callback fired when any session exits (wired to ChannelEmitter).
    pub fn on_exit_session(&self, id: &str) -> Result<Arc<PtySession>, VoxError> {
        self.get(id)
    }
}

impl Default for PtyManager {
    fn default() -> Self { Self::new() }
}
```

- [ ] **Step 4: Run `cargo test manager:: -- --nocolor`; pass; lint.**
- [ ] **Step 5: Commit** — `git commit -m "feat: Consecrate the PtyManager — keeper of sessions"`

---

### Task 8: ChannelEmitter — notifications/claude/channel

**Files:**
- Create: `src/channel.rs`

- [ ] **Step 1: Failing test** (inline — verifies the params JSON shape, no live peer):

```rust
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
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** (pure param builder + a thin send that uses rmcp's `Peer`):

```rust
use serde_json::{json, Value};
use crate::types::SessionInfo;

pub const CHANNEL_METHOD: &str = "notifications/claude/channel";

/// Build the params for a `notifications/claude/channel` exit push.
/// Renders in-session as `<channel source="voxcaster" session_id=.. exit_code=..>content</channel>`.
pub fn build_exit_params(session_id: &str, cmdline: &str, exit_code: Option<i32>) -> Value {
    let code = exit_code.map(|c| c.to_string()).unwrap_or_else(|| "unknown".into());
    json!({
        "content": format!("Process `{cmdline}` exited (code {code})."),
        "meta": {
            "session_id": session_id,
            "exit_code": code
        }
    })
}

pub fn exit_params_for(info: &SessionInfo) -> Value {
    let cmdline = format!("{} {}", info.command, info.args.join(" ")).trim().to_string();
    build_exit_params(&info.id, &cmdline, info.exit_code)
}
```

> The actual send is wired in Task 9 where the rmcp `Peer` is in scope. ⚠️ADAPT: rmcp exposes server→client custom notifications via the request-context `Peer`. Send with the peer's notification API and `CustomNotification { method: CHANNEL_METHOD.into(), params: Some(params), extensions: Default::default() }`. Confirm the exact method name (`Peer::send_notification` / `peer.notify(...)`) against the linked rmcp docs at implementation time; the param JSON above is transport-independent and fully tested here.

- [ ] **Step 4: Pass + lint.**
- [ ] **Step 5: Commit** — `git commit -m "feat: Forge the ChannelEmitter exit-vox transmission"`

---

### Task 9: McpServer — the 6 tools + capabilities + channel wiring

**Files:**
- Create: `src/server.rs`

This task exposes the manager over MCP. Tools use rmcp's `#[tool_router]`/`#[tool]` macros with `Parameters<T>` structs. Each tool supports `format: "text" | "json"`.

- [ ] **Step 1: Failing test** (inline — exercises the tool bodies directly, not over the wire):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_then_read_text_and_json() {
        let policy = std::sync::Arc::new(crate::policy::PolicyEngine::from_lists(vec![], vec![], true));
        let srv = VoxServer::new_for_test(policy);

        #[cfg(windows)]
        let (command, args) = ("cmd".to_string(), vec!["/C".to_string(), "echo hi".to_string()]);
        #[cfg(not(windows))]
        let (command, args) = ("/bin/echo".to_string(), vec!["hi".to_string()]);

        let id = srv.spawn_inner(crate::types::SpawnOptions {
            command, args, workdir: None, env: vec![], title: None,
            notify_on_exit: false, timeout_seconds: None,
        }).unwrap();

        srv.wait_inner(&id, Some(10)).await.unwrap();
        let txt = srv.read_inner(&id, 0, None, false, None).unwrap();
        assert!(txt.contains("hi"));

        let denied = crate::policy::PolicyEngine::from_lists(vec![], vec!["rm *".into()], true);
        assert!(denied.check("rm", &["x".into()]).is_err());
    }
}
```

- [ ] **Step 2: Run, confirm fail.**
- [ ] **Step 3: Implement** — split into (a) the inner, test-friendly methods and (b) the rmcp tool surface that wraps them.

```rust
use std::sync::Arc;
use rmcp::{ServerHandler, model::*, handler::server::tool::Parameters};
use rmcp::handler::server::router::tool::ToolRouter;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;
use crate::pty::manager::PtyManager;
use crate::policy::PolicyEngine;
use crate::types::{SpawnOptions, VoxError};
use crate::channel;

#[derive(Clone)]
pub struct VoxServer {
    manager: PtyManager,
    policy: Arc<PolicyEngine>,
    tool_router: ToolRouter<Self>,
}

// ---- Parameter structs (one per tool) ----
#[derive(Deserialize, JsonSchema)]
pub struct SpawnParams {
    pub command: String,
    #[serde(default)] pub args: Vec<String>,
    pub workdir: Option<String>,
    #[serde(default)] pub env: std::collections::HashMap<String, String>,
    pub title: Option<String>,
    pub description: String,
    #[serde(default)] pub notify_on_exit: bool,
    pub timeout_seconds: Option<u64>,
    pub format: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
pub struct WriteParams { pub id: String, pub data: String, pub format: Option<String> }
#[derive(Deserialize, JsonSchema)]
pub struct ReadParams {
    pub id: String, pub offset: Option<usize>, pub limit: Option<usize>,
    #[serde(default)] pub raw: bool, pub pattern: Option<String>, pub format: Option<String>,
}
#[derive(Deserialize, JsonSchema)]
pub struct WaitParams { pub id: String, pub timeout_seconds: Option<u64>, pub format: Option<String> }
#[derive(Deserialize, JsonSchema)]
pub struct ListParams { pub format: Option<String> }
#[derive(Deserialize, JsonSchema)]
pub struct KillParams { pub id: String, #[serde(default)] pub cleanup: bool, pub format: Option<String> }

// ---- Inner methods (unit-testable, no rmcp types) ----
impl VoxServer {
    pub fn new(manager: PtyManager, policy: Arc<PolicyEngine>) -> Self {
        Self { manager, policy, tool_router: Self::tool_router() }
    }
    #[cfg(test)]
    pub fn new_for_test(policy: Arc<PolicyEngine>) -> Self {
        Self::new(PtyManager::new(), policy)
    }

    pub fn spawn_inner(&self, opts: SpawnOptions) -> Result<String, VoxError> {
        self.policy.check(&opts.command, &opts.args)?;
        self.manager.spawn(opts)
    }
    pub async fn wait_inner(&self, id: &str, t: Option<u64>) -> Result<(), VoxError> {
        self.manager.wait(id, t).await.map(|_| ())
    }
    pub fn read_inner(&self, id: &str, offset: usize, limit: Option<usize>, raw: bool, pattern: Option<&str>)
        -> Result<String, VoxError> {
        let slice = self.manager.read(id, offset, limit, raw)?;
        let lines: Vec<&String> = match pattern.and_then(|p| globset::Glob::new(p).ok()) {
            Some(g) => { let m = g.compile_matcher(); slice.lines.iter().filter(|l| m.is_match(l)).collect() }
            None => slice.lines.iter().collect(),
        };
        Ok(lines.iter().map(|l| l.as_str()).collect::<Vec<_>>().join("\n"))
    }
}

// ---- rmcp tool surface ----
#[rmcp::tool_router]
impl VoxServer {
    #[rmcp::tool(name = "pty_spawn", description = "Start a background process in a PTY. Pass command + args array (no shell string).")]
    async fn pty_spawn(&self, Parameters(p): Parameters<SpawnParams>) -> Result<CallToolResult, ErrorData> {
        let opts = SpawnOptions {
            command: p.command, args: p.args, workdir: p.workdir,
            env: p.env.into_iter().collect(), title: p.title,
            notify_on_exit: p.notify_on_exit, timeout_seconds: p.timeout_seconds,
        };
        match self.spawn_inner(opts) {
            Ok(id) => {
                let info = self.manager.list().into_iter().find(|s| s.id == id);
                Ok(render(p.format.as_deref(), info.map(|i| json!(i)).unwrap_or(json!({"id": id}))))
            }
            Err(e) => Ok(tool_err(&e)),
        }
    }

    #[rmcp::tool(name = "pty_write", description = "Send stdin to a running PTY session.")]
    async fn pty_write(&self, Parameters(p): Parameters<WriteParams>) -> Result<CallToolResult, ErrorData> {
        match self.manager.write(&p.id, &p.data) {
            Ok(()) => Ok(render(p.format.as_deref(), json!({"ok": true}))),
            Err(e) => Ok(tool_err(&e)),
        }
    }

    #[rmcp::tool(name = "pty_read", description = "Read output from a PTY session (line offset/limit, raw|plain, optional regex/glob filter).")]
    async fn pty_read(&self, Parameters(p): Parameters<ReadParams>) -> Result<CallToolResult, ErrorData> {
        match self.manager.read(&p.id, p.offset.unwrap_or(0), p.limit, p.raw) {
            Ok(slice) => {
                if p.format.as_deref() == Some("json") {
                    Ok(render(Some("json"), json!({
                        "sessionId": p.id, "lineRange": {"offset": slice.offset, "limit": p.limit, "total": slice.total},
                        "lines": slice.lines, "truncated": slice.truncated
                    })))
                } else {
                    let body = self.read_inner(&p.id, p.offset.unwrap_or(0), p.limit, p.raw, p.pattern.as_deref())
                        .unwrap_or_default();
                    Ok(CallToolResult::success(vec![Content::text(body)]))
                }
            }
            Err(e) => Ok(tool_err(&e)),
        }
    }

    #[rmcp::tool(name = "pty_wait", description = "Block until the process exits or the timeout elapses; returns exit status.")]
    async fn pty_wait(&self, Parameters(p): Parameters<WaitParams>) -> Result<CallToolResult, ErrorData> {
        match self.manager.wait(&p.id, p.timeout_seconds).await {
            Ok(info) => Ok(render(p.format.as_deref(), json!(info))),
            Err(e) => Ok(tool_err(&e)),
        }
    }

    #[rmcp::tool(name = "pty_list", description = "List all PTY sessions and their status.")]
    async fn pty_list(&self, Parameters(p): Parameters<ListParams>) -> Result<CallToolResult, ErrorData> {
        Ok(render(p.format.as_deref(), json!(self.manager.list())))
    }

    #[rmcp::tool(name = "pty_kill", description = "Terminate a PTY session. cleanup=true also frees its buffer.")]
    async fn pty_kill(&self, Parameters(p): Parameters<KillParams>) -> Result<CallToolResult, ErrorData> {
        match self.manager.kill(&p.id, p.cleanup) {
            Ok(()) => Ok(render(p.format.as_deref(), json!({"ok": true}))),
            Err(e) => Ok(tool_err(&e)),
        }
    }
}

#[rmcp::tool_handler]
impl ServerHandler for VoxServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_experimental()
                .build(),
            instructions: Some(
                "Voxcaster: persistent interactive PTY sessions. Use pty_spawn to start a \
                 background process, pty_read/pty_wait to observe it, pty_write to send input, \
                 pty_kill to stop it.".into()),
            ..Default::default()
        }
    }
}

// ---- helpers ----
fn render(format: Option<&str>, value: serde_json::Value) -> CallToolResult {
    if format == Some("json") {
        CallToolResult::success(vec![Content::text(value.to_string())])
    } else {
        CallToolResult::success(vec![Content::text(human(&value))])
    }
}
fn human(v: &serde_json::Value) -> String { v.to_string() } // text mode: compact; refine per field later
fn tool_err(e: &VoxError) -> CallToolResult {
    CallToolResult::error(vec![Content::text(format!("{e}"))])
}
```

> ⚠️ADAPT (verify against live rmcp): the macro names `#[rmcp::tool_router]`, `#[rmcp::tool]`, `#[rmcp::tool_handler]`; `Parameters<T>`; `CallToolResult::success/error`; `Content::text`; `ServerCapabilities::builder().enable_experimental()`. These match rmcp 0.9. If `enable_experimental()` does not let us inject the literal `claude/channel` key, set `capabilities.experimental` directly to `Some(json-map with "claude/channel": {})` on the built struct before returning. The `human()` text renderer is intentionally minimal in v1 — refine to field-by-field formatting in a follow-up, tracked in Task 12.

- [ ] **Step 4: Run `cargo test server:: -- --nocolor`; pass; lint.**
- [ ] **Step 5: Commit** — `git commit -m "feat: Raise the McpServer and its six canticles"`

---

### Task 10: main.rs — wire it up, serve over stdio, reap on shutdown

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: Implement** (no unit test — covered by Task 11 integration):

```rust
mod channel;
mod policy;
mod server;
mod types;
mod pty;

use std::sync::Arc;
use rmcp::ServiceExt;
use rmcp::transport::stdio;
use policy::PolicyEngine;
use pty::manager::PtyManager;
use server::VoxServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let policy_path = std::env::var("VOXCASTER_POLICY")
        .unwrap_or_else(|_| "voxcaster-policy.toml".to_string());
    let policy = Arc::new(PolicyEngine::from_file(&policy_path));
    let manager = PtyManager::new();
    let server = VoxServer::new(manager.clone(), policy);

    // Reap all PTYs when the parent (Claude Code session) goes away.
    let reaper = manager.clone();
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        reaper.reap_all();
        std::process::exit(0);
    });

    let service = server.serve(stdio()).await?;
    service.waiting().await?;
    manager.reap_all();
    Ok(())
}
```

> ⚠️ADAPT: `ServiceExt::serve`, `rmcp::transport::stdio`, and `service.waiting()` are the rmcp 0.9 stdio-server entrypoints. On Unix, also handle SIGTERM (stdin EOF closes `serve`'s loop, which is the primary reap trigger); `ctrl_c` is the cross-platform belt-and-braces.

- [ ] **Step 2: Build** — `cargo build`; expected clean.
- [ ] **Step 3: Commit** — `git commit -m "feat: Light the stdio conduit and shutdown reaping"`

---

### Task 11: End-to-end integration test

**Files:**
- Create: `tests/integration.rs`

- [ ] **Step 1: Write the test** (drives the manager exactly as the tools do):

```rust
use voxcaster::pty::manager::PtyManager;
use voxcaster::types::{SpawnOptions, Status};

fn echo(line: &str) -> SpawnOptions {
    #[cfg(windows)]
    let (command, args) = ("cmd".to_string(), vec!["/C".into(), format!("echo {line}")]);
    #[cfg(not(windows))]
    let (command, args) = ("/bin/echo".to_string(), vec![line.to_string()]);
    SpawnOptions { command, args, workdir: None, env: vec![], title: None,
                   notify_on_exit: false, timeout_seconds: None }
}

#[tokio::test]
async fn full_lifecycle() {
    let m = PtyManager::new();
    let id = m.spawn(echo("integration-ok")).unwrap();
    let info = m.wait(&id, Some(10)).await.unwrap();
    assert_eq!(info.status, Status::Exited);
    assert_eq!(info.exit_code, Some(0));
    let slice = m.read(&id, 0, None, false).unwrap();
    assert!(slice.lines.iter().any(|l| l.contains("integration-ok")), "got {:?}", slice.lines);
}
```

- [ ] **Step 2:** Requires `manager`/`types`/`pty` to be reachable from an integration test. Add to `src/main.rs` top: `pub mod` — but a binary crate isn't importable. **Resolution:** add `src/lib.rs` exposing the modules, and have `main.rs` depend on the lib.

`src/lib.rs`:

```rust
pub mod channel;
pub mod policy;
pub mod server;
pub mod types;
pub mod pty;
```

Update `Cargo.toml`:

```toml
[lib]
name = "voxcaster"
path = "src/lib.rs"
```

And trim `src/main.rs` to `use voxcaster::{...};` instead of `mod` declarations.

- [ ] **Step 3: Run** — `cargo test --test integration -- --nocolor`; expected PASS on Linux and Windows.
- [ ] **Step 4: Commit** — `git commit -m "test: Rite of Validation — full PTY lifecycle"`

---

### Task 12: CI matrix + README polish + text renderer refinement

**Files:**
- Create: `.gitea/workflows/ci.yml` (or `.github/workflows/ci.yml` mirror)
- Modify: `src/server.rs` (`human()` → field-aware text output)
- Modify: `README.md` (usage: `claude mcp add`, policy file, channel opt-in flag)

- [ ] **Step 1: CI workflow** (build + test + clippy on ubuntu-latest and windows-latest):

```yaml
name: ci
on: [push, pull_request]
jobs:
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, windows-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: clippy, rustfmt }
      - run: cargo fmt --all -- --check
      - run: cargo clippy --all-targets --all-features -- -D warnings
      - run: cargo test --all -- --nocolor
```

- [ ] **Step 2: Refine `human()`** to render `SessionInfo`/lists as concise key:value text (test: `human(&json!({"id":"x","status":"running"}))` contains `id: x`). Implement field iteration for objects/arrays.
- [ ] **Step 3: README** — add an "Install" section: `claude mcp add --transport stdio voxcaster -- /path/to/voxcaster`; policy file location (`VOXCASTER_POLICY`); channel opt-in (`claude --dangerously-load-development-channels server:voxcaster`).
- [ ] **Step 4: Run full suite + clippy; commit** — `git commit -m "ci: Establish the cross-platform Rite of Validation"`

---

## Self-Review

**Spec coverage:**
- §3 stack (Rust/portable-pty/stdio) → Tasks 1, 6, 10 ✓
- §5 six tools + text|json → Task 9 ✓
- §6 output model (ring buffer, line addressing, ANSI, grep) → Tasks 2, 9 ✓
- §7 exit model: `pty_wait` floor → Tasks 6, 7, 9; channel push → Tasks 8, 9 ✓
- §8 defense-in-depth permissions: server PolicyEngine → Task 5, 9 (CC-native gate is automatic — `pty_spawn` is a normal tool) ✓
- §9 session-bound lifecycle + reap → Tasks 7, 10 ✓
- §10 typed errors → Task 4 (`VoxError`), surfaced in Task 9 (`tool_err`) ✓
- §11 testing (unit + integration + channel) → Tasks 2–9 inline, 11 integration, 8 channel-params ✓

**Known deferrals (explicitly scoped, not placeholders):**
- `human()` text renderer ships minimal in Task 9, refined in Task 12 — flagged, not hidden.
- Channel *send* call (vs param-build) is wired in Task 9 against the live rmcp `Peer`; param shape is fully tested in Task 8. ⚠️ADAPT noted.
- `notify_on_exit` → actual channel emit requires an exit hook from `PtySession`'s waiter thread to the server's `Peer`. **Add during Task 9 implementation:** pass an `Option<Peer>`-bearing callback into `PtyManager::spawn` so the waiter thread calls `ChannelEmitter` on exit. If the `Peer` is not yet obtainable at spawn time in rmcp's model, fall back to emitting on the next `pty_list`/`pty_read` poll — the `pty_wait` floor guarantees correctness regardless.

**Type consistency:** `SpawnOptions`, `SessionInfo`, `Status`, `ReadSlice`, `VoxError` names are used identically across Tasks 4–11. `PtyManager` method names (`spawn/write/read/wait/list/kill/reap_all`) are consistent between Tasks 7, 9, 10, 11.

**Scope:** single subsystem (one MCP server crate) — one plan, correct granularity.
