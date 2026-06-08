# Voxcaster

> *The channel to the machine spirit.*

A cross-platform **Model Context Protocol (MCP) server** that gives an AI agent
**persistent, interactive pseudo-terminal (PTY) sessions** — run a dev server,
watcher, or REPL in the background, send it input, read its output on demand,
and react hands-free when it exits.

It fills the gap left by the synchronous `bash` tool, which blocks for the
lifetime of the command and offers no interactive stdin or true TTY.

## Status

Working implementation, actively used. All 6 tools are implemented in Rust;
tests pass on Windows and Linux, including end-to-end coverage of the channel
exit-push. Not yet production-hardened; breaking changes may occur before a
first tagged release.

## Quick start

### Build

```sh
cargo build --release
# produces: target/release/voxcaster   (target/release/voxcaster.exe on Windows)
```

### Register with Claude Code

```sh
# Linux / macOS
claude mcp add --transport stdio voxcaster -- /absolute/path/to/voxcaster

# Windows
claude mcp add --transport stdio voxcaster -- C:\absolute\path\to\voxcaster.exe
```

### Local install (Windows)

Builds the release binary, stages it to `%USERPROFILE%\.local\bin`, and copies
the usage skill into `~/.claude/skills/voxcaster` — one command from the repo root:

```powershell
.\scripts\install-local.ps1
```

## Tools

| Tool | Purpose |
|------|---------|
| `pty_spawn` | Start a background process (argv array — no shell string) |
| `pty_write` | Send stdin to a running session |
| `pty_read` | Read output (line offset/limit · raw\|plain · glob filter · text\|json) |
| `pty_wait` | Block until the process exits or a timeout elapses |
| `pty_list` | List sessions and their status |
| `pty_kill` | Terminate a session |

## Usage skill

`skills/voxcaster/SKILL.md` is a Claude Code skill that teaches agents when and
how to use the 6 tools — covering core patterns, anti-patterns, and the
hands-free channel exit notification. `install-local.ps1` copies it alongside
the binary; or copy it manually into `~/.claude/skills/voxcaster/SKILL.md`.

## Channel exit-push

When a session is spawned with `notify_on_exit: true`, Voxcaster pushes a
`notifications/claude/channel` event into the running Claude Code session as
the process exits — so the agent reacts hands-free without polling:

```
<channel source="voxcaster" session_id="pty_…" exit_code="0">
Process `…` exited (code 0).
</channel>
```

This requires Claude Code's channels research preview. Enable it per session
with the development flag:

```sh
claude --dangerously-load-development-channels server:voxcaster
```

Requires Claude Code ≥ v2.1.80 and claude.ai/Console authentication; not
available on Bedrock/Vertex/Foundry. When the channel is not active, the push
is silently skipped — **`pty_wait` remains the guaranteed completion path** for
every session regardless.

## Command policy

A built-in **catastrophic floor** is always enforced — blocking recursive
root/home deletion, `mkfs`/`dd` to raw disks, Windows `format`/`diskpart`/
`vssadmin delete`, pipe-to-shell RCE, fork bombs, and `shutdown`/`reboot`.
It is an accident backstop, not a security sandbox.

For project-specific rules, set `VOXCASTER_POLICY=/path/to/voxcaster-policy.toml`
(see [`voxcaster-policy.example.toml`](voxcaster-policy.example.toml)). Without
it, a permissive default applies (allow-by-default, floor still enforced) and a
warning is printed to stderr.

When running with `--dangerously-skip-permissions`, set `allow_by_default = false`
with an explicit `allow` list — Voxcaster's policy is then the only gate.

## Security

- `pty_spawn` takes an **argv array**, never a shell string — no shell-injection
  surface by construction.
- **Defense-in-depth:** Claude Code's native per-tool permission prompt, the
  catastrophic baseline floor, and an optional server-side allow/deny policy
  file — three independent gates, each enforced even if the others are bypassed.

## Windows — ConPTY redistributable

Voxcaster bundles a modern ConPTY redistributable and self-stages it to
`%LOCALAPPDATA%\voxcaster\conpty` on first run — no manual file placement
required. This ensures `portable-pty` uses a known-good ConPTY host even on
Windows 10 LTSC/IoT images where the inbox ConPTY is broken (every child exits
`0xC0000142`).

The bundled redist is the
[`Microsoft.Windows.Console.ConPTY`](https://www.nuget.org/packages/Microsoft.Windows.Console.ConPTY)
NuGet package — MIT licensed, see `assets/win-x64/LICENSE-ConPTY.txt`.

## At a glance

| | |
|---|---|
| **Language** | Rust — single static binary, no runtime to provision |
| **PTY backend** | [`portable-pty`](https://crates.io/crates/portable-pty) — Unix `forkpty`, Windows ConPTY, macOS |
| **Transport** | MCP stdio (Claude Code spawns it as a subprocess) |
| **Platforms** | Linux (`musl` static), Windows |

## Lineage

Capability modelled on the MIT-licensed
[`opencode-pty`](https://github.com/shekohex/opencode-pty) plugin, used **only
as a reference architecture**. Voxcaster is a clean-room reimplementation — no
code copied — that drops the opencode/Bun coupling and web-server attack surface,
and adds cross-platform Rust, layered exit signalling, and defense-in-depth
permissions.

## Development

This GitHub repo is a downstream fork of an upstream private repository.
The only content unique to this fork is `.github/workflows/`.

To pull upstream changes into the fork:

```sh
git fetch origin          # fetch from upstream
git rebase origin/main    # replay the .github/ overlay on top
git push github main --force-with-lease
```

The overlay commit adds only new files and rebases cleanly onto any upstream state.

## License

MIT
