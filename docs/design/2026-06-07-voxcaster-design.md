# Design — Voxcaster (fleet-native PTY MCP server)

**Date:** 2026-06-07
**Status:** Design approved in brainstorm; pending written-spec review
**Author:** Warden Verity (brainstorm with the Commander)

> *Voxcaster — the channel to the machine spirit.* A binharic conduit through
> which the agent transmits to, listens to, and tends the running processes in
> its care.

## 1. Summary

A cross-platform Model Context Protocol (MCP) server that gives an AI agent
**persistent, interactive pseudo-terminal (PTY) sessions** — run a dev server,
watcher, or REPL in the background, send it input, read its output on demand,
and learn when it exits. It fills the gap left by the synchronous `bash` tool,
which blocks and gives no interactive stdin or true TTY.

The capability is modelled on the MIT-licensed `opencode-pty` plugin, used
**only as a reference architecture (STC)**. No code is copied; this is a
clean-room reimplementation, so no attribution obligation attaches and the
opencode/Bun coupling is discarded.

- **Repo:** `StellarFleetOps/voxcaster`
- **Binary:** `voxcaster` (short alias `vox`)

## 2. Goals / Non-goals

**Goals**
- Run on the Commander's Windows box **and** the fleet's Ubuntu/Debian hosts —
  one codebase, single static binary per OS.
- Persistent background processes with interactive stdin and real TTY semantics.
- Reliable completion signalling that works on every host and auth mode.
- Defense-in-depth command permissioning that holds even under
  `--dangerously-skip-permissions`.

**Non-goals (v1)**
- No web/terminal UI (the `opencode-pty` web server was the source of its one
  HIGH security finding; cut entirely).
- No cross-session process persistence (no daemon). PTYs are session-bound.
- No `notifications/progress` reporting (confirmed unsupported by Claude Code,
  issue #4157).

## 3. Stack & platform

- **Language:** Rust (primary fleet language; single static binary, no runtime
  to provision on Debian hosts — matches the existing CLI-on-PATH fleet pattern).
- **PTY backend:** `portable-pty` (wezterm) — abstracts Unix `forkpty`, Windows
  **ConPTY**, and macOS behind one API. Strongest Windows TTY story available.
- **Transport:** MCP **stdio** (Claude Code spawns the server as a subprocess).
  stdio is also the transport the channel push requires — no network surface.
- **Distribution:** `musl` static ELF for Debian/Ubuntu; `.exe` for Windows.

## 4. Architecture

```
Claude Code session
  └─ spawns voxcaster  (stdio subprocess, one per session)
       ├─ McpServer            JSON-RPC over stdio; tool dispatch + channel emit
       ├─ PtyManager           owns all sessions, lifecycle FSM, reaping
       │    └─ PtySession*     { portable-pty child, RingBuffer, status, meta }
       ├─ PolicyEngine         allow/deny glob matcher (always enforced)
       └─ ChannelEmitter       optional <channel> push on exit (isolation seam)
```

Each unit has one purpose and a narrow interface:

- **McpServer** — speaks MCP stdio JSON-RPC: advertises capabilities
  (`tools`, `experimental["claude/channel"]`), routes `tools/call`, serialises
  results. Knows nothing about PTY internals.
- **PtyManager** — the only owner of sessions. `spawn / write / read / wait /
  list / kill`, plus `reap_all()` on shutdown. Transport- and protocol-agnostic
  (clean enough to test in isolation; *not* pre-abstracted for a future daemon —
  YAGNI).
- **PtySession** — a single child process + its `RingBuffer` + status. Lifecycle
  FSM: `running → killing → killed | exited`. Carries per-session timeout.
- **PolicyEngine** — pure function over `(command, args[], policy)` → `allow |
  deny`. No I/O beyond loading the policy file once at startup.
- **ChannelEmitter** — wraps the `notifications/claude/channel` emit behind a
  single seam so the research-preview contract can change without touching the
  rest of the server. Disabled cleanly when channels aren't active.

## 5. Tool surface (6 tools)

All tools accept optional `format: "text" | "json"` (default `text`).
`text` is concise and token-light; `json` returns a structured envelope.

| Tool | Args | Returns |
|------|------|---------|
| `pty_spawn` | `command`, `args[]`, `workdir?`, `env?`, `title?`, `description`, `notifyOnExit?`, `timeoutSeconds?` | session id, pid, status |
| `pty_write` | `id`, `data` | ok / error |
| `pty_read` | `id`, `offset?`, `limit?`, `raw?`, `pattern?`, `format?` | output lines or regex matches |
| `pty_wait` | `id`, `timeoutSeconds?` | blocks until exit-or-timeout → status, exitCode, new output |
| `pty_list` | `format?` | sessions + status |
| `pty_kill` | `id`, `cleanup?` | ok / error |

**`pty_spawn` takes an argv array, never a shell string** — no shell-injection
surface by construction (the one design virtue carried verbatim from the STC).

**`json` envelope shape (read):**
```json
{
  "sessionId": "pty_1a2b3c4d",
  "status": "running",
  "exitCode": null,
  "lineRange": { "offset": 0, "limit": 50, "total": 812 },
  "lines": ["...", "..."],
  "truncated": false
}
```

## 6. Output model

- **RingBuffer per session** — bounded scrollback, default cap ~256 KB **or**
  ~5,000 lines (configurable), oldest dropped on overflow. Bounded memory.
- **Line-addressed reads** — `offset` / `limit` by line, not byte.
- **ANSI** — stored **raw**; returned **stripped by default**, `raw: true` to
  get escape codes (for driving a TUI).
- **Regex grep folded into `pty_read`** via `pattern` — no separate tool.

## 7. Exit / completion model (layered)

The reason this beats `bash` is async background work, so completion signalling
is the core UX. Two layers:

1. **Floor — `pty_wait` (always present, every host/auth/mode).**
   Blocking tool: returns when the process exits or `timeoutSeconds` elapses,
   reporting exit code + any new output. No preview dependency. This is the
   guaranteed path.

2. **Enhancement — channel push (when channels are enabled).**
   The server declares `capabilities.experimental["claude/channel"] = {}` and,
   on PTY exit (when `notifyOnExit`), fires a `notifications/claude/channel`
   JSON-RPC notification. It lands in the session as:
   ```
   <channel source="voxcaster" session_id="pty_1a2b3c4d" exit_code="0">
   Process `npm run dev` exited (code 0).
   </channel>
   ```
   The agent reacts hands-free, even mid-other-work. Same single stdio process —
   verified against the fakechat reference: one server can be both a tool-server
   and a channel.

**Channel caveats (designed around, not bet on):**
- Research preview (CC ≥ v2.1.80); capability strings unversioned, contract may
  change → fully isolated in `ChannelEmitter`; if it breaks, one file changes
  and the `pty_wait` floor is untouched.
- Custom channels need per-session opt-in:
  `claude --dangerously-load-development-channels server:voxcaster`.
- Not available on Bedrock / Vertex / Foundry. Fleet hosts on those auth modes
  simply fall back to the floor.
- Delivery: pushes **queue if the agent is busy** and arrive at the next turn
  boundary; they do not interrupt a running turn. Acceptable for "finished".

## 8. Permission model (defense-in-depth)

Two independent gates; remediates the audit's HIGH finding at the design level.

1. **Claude Code native** — `pty_spawn` is a normal MCP tool, so CC's per-tool
   permission prompt gates it interactively. Correct UX when a human is present.
2. **Server-side PolicyEngine — always enforced**, even under
   `--dangerously-skip-permissions`. Loaded from `voxcaster-policy.toml`:
   ```toml
   allow = ["npm *", "cargo *", "git *", "go *"]
   deny  = ["rm -rf /*", "* | sh", "curl * | *"]
   ```
   Glob match on `command + args`. Default posture configurable
   (deny-by-default vs allow-by-default); deny rules always win. This is the
   gate that protects unattended Debian hosts.

## 9. Lifecycle & cleanup (session-bound)

- PTYs are children of the per-session stdio server process.
- On session end, Claude Code sends SIGTERM / closes stdin → server runs
  `PtyManager::reap_all()`, killing every child PTY. **No orphans, no daemon,
  no reaping heuristics.**
- Per-session `timeoutSeconds` auto-kills a runaway PTY.
- `pty_kill` for explicit termination; `cleanup: true` also frees the buffer.

## 10. Error handling

- Typed errors throughout (`thiserror`); no silent failures. Every tool returns
  a structured error (`{ error, detail }` in `json` mode) rather than a bare
  string when something fails.
- Spawn failures (bad binary, denied by policy, bad workdir) return a clear,
  actionable message naming the cause.
- PolicyEngine denial returns a distinct error so the agent can distinguish
  "blocked by policy" from "command failed".

## 11. Testing

- **Unit:** PolicyEngine glob matching (allow/deny/precedence); RingBuffer
  overflow + line addressing; FSM transitions; ANSI strip.
- **Integration:** spawn a real short-lived process (`echo`, a sleep loop),
  read incrementally, `pty_wait` for exit, assert exit code + buffered output.
  Cross-platform matrix: Linux + Windows runners.
- **Channel:** unit-test the JSON-RPC notification serialisation against the
  documented `notifications/claude/channel` shape; manual end-to-end behind the
  dev flag.

## 12. Open items for the plan phase

- Confirm `portable-pty` API surface + Debian `musl` cross-compile toolchain.
- Choose Rust MCP layer: `rmcp` (official) vs a thin hand-rolled JSON-RPC stdio
  loop — the latter may be cleaner for emitting the custom channel notification.
- Final RingBuffer cap defaults; policy default posture (deny- vs allow-by-default).
