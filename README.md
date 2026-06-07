# Voxcaster

> *The channel to the machine spirit.*

A cross-platform **Model Context Protocol (MCP) server** that gives an AI agent
**persistent, interactive pseudo-terminal (PTY) sessions** — run a dev server,
watcher, or REPL in the background, send it input, read its output on demand,
and learn when it exits.

It fills the gap left by the synchronous `bash` tool, which blocks for the
lifetime of the command and offers no interactive stdin or true TTY.

## Status

**Working implementation — not yet released.**
All 6 tools are implemented in Rust; 15+ tests pass locally on Windows and Linux.
The architecture is documented in
[`docs/design/2026-06-07-voxcaster-design.md`](docs/design/2026-06-07-voxcaster-design.md).
Not production-hardened; breaking changes may occur before a versioned release.

## Install / Run

### Build

```sh
cargo build --release
# produces: target/release/voxcaster   (target/release/voxcaster.exe on Windows)
```

### Register with Claude Code

```sh
claude mcp add --transport stdio voxcaster -- /absolute/path/to/voxcaster
# Windows:
claude mcp add --transport stdio voxcaster -- C:\absolute\path\to\voxcaster.exe
```

### Command policy

Set `VOXCASTER_POLICY=/path/to/voxcaster-policy.toml` (see
[`voxcaster-policy.example.toml`](voxcaster-policy.example.toml)).
Without it a permissive default applies and a warning is printed to stderr.

### Channel exit-push (optional, research preview)

Opt in per session with:

```sh
claude --dangerously-load-development-channels server:voxcaster
```

> **Note:** exit-push channel wiring is not yet implemented.
> `pty_wait` is the guaranteed completion path for all sessions.

### Windows note — ConPTY redistributable

On Windows 10 LTSC/IoT (and other images where the inbox ConPTY is broken),
place a modern `conpty.dll` and `OpenConsole.exe` **next to `voxcaster.exe`**.
Obtain them from the
[`Microsoft.Windows.Console.ConPTY`](https://www.nuget.org/packages/Microsoft.Windows.Console.ConPTY)
NuGet redistributable — download the `.nupkg`, unzip it, and copy:

```
runtimes/win-x64/native/conpty.dll
build/native/runtimes/x64/OpenConsole.exe
```

`portable-pty` auto-prefers a co-located `conpty.dll` over the inbox one.
Automatic bundling is planned for a future release.

## At a glance

- **Language:** Rust — single static binary, no runtime to provision.
- **PTY backend:** [`portable-pty`](https://crates.io/crates/portable-pty) —
  Unix `forkpty`, Windows ConPTY, macOS, behind one API.
- **Transport:** MCP stdio (Claude Code spawns it as a subprocess).
- **Platforms:** Linux (Debian/Ubuntu, `musl` static) and Windows.

## Tools

| Tool | Purpose |
|------|---------|
| `pty_spawn` | Start a background process (argv array — no shell string) |
| `pty_write` | Send stdin to a running session |
| `pty_read` | Read output (line offset/limit · raw\|plain · regex filter · text\|json) |
| `pty_wait` | Block until the process exits or a timeout elapses |
| `pty_list` | List sessions and their status |
| `pty_kill` | Terminate a session |

On process exit, Voxcaster can optionally push a `<channel source="voxcaster">`
event into the session (Claude Code channels) so the agent reacts hands-free.
A blocking `pty_wait` is always available as the guaranteed completion path.

## Security

- `pty_spawn` passes an **argv array**, never a shell string — no shell-injection
  surface by construction.
- **Defense-in-depth permissioning:** Claude Code's native per-tool prompt *plus*
  a server-side allow/deny policy that is enforced even under
  `--dangerously-skip-permissions`.

## Lineage

Capability modelled on the MIT-licensed
[`opencode-pty`](https://github.com/shekohex/opencode-pty) plugin, used **only as
a reference architecture**. Voxcaster is a clean-room reimplementation — no code
copied — that discards the opencode/Bun coupling and the web-server attack
surface, and adds cross-platform Rust, layered exit signalling, and
defense-in-depth permissions.

## License

TBD.
