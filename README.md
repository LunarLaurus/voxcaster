# Voxcaster

> *The channel to the machine spirit.*

A cross-platform **Model Context Protocol (MCP) server** that gives an AI agent
**persistent, interactive pseudo-terminal (PTY) sessions** — run a dev server,
watcher, or REPL in the background, send it input, read its output on demand,
and learn when it exits.

It fills the gap left by the synchronous `bash` tool, which blocks for the
lifetime of the command and offers no interactive stdin or true TTY.

## Status

**Design stage.** The architecture and tool surface are specified in
[`docs/design/2026-06-07-voxcaster-design.md`](docs/design/2026-06-07-voxcaster-design.md).
No implementation has begun.

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
