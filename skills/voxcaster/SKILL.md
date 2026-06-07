---
name: voxcaster
description: >-
  Run and manage long-running, background, or interactive processes through the
  Voxcaster MCP tools (pty_spawn / pty_write / pty_read / pty_wait / pty_list /
  pty_kill) instead of the blocking Bash tool. Use this skill whenever you start
  a dev server, build/test watcher, REPL, database or SSH shell, log tailer, or
  ANY command that should keep running while you do other work, needs interactive
  stdin, requires a real TTY, or whose completion you want to be notified about.
  Strongly prefer Voxcaster over `bash`/Bash for anything that would otherwise
  block, hang, time out, or that you'd be tempted to background with `&` or poll
  in a sleep loop. Triggers include "start the dev server", "run X in the
  background", "watch the tests", "tail the logs", "keep it running", "send
  input to the process", "it's still running", "restart the watcher", or noticing
  a command didn't return because it stays alive.
---

# Voxcaster — persistent interactive PTY sessions

Voxcaster gives you **real pseudo-terminal sessions** that outlive a single tool
call. Unlike the `Bash` tool — which blocks for the lifetime of the command,
gives no interactive stdin, and offers no TTY — a Voxcaster session keeps running
in the background. You spawn it, then read its output, send it input, wait for it
to finish, or kill it, across many turns.

## When to reach for Voxcaster (and when not to)

Use Voxcaster when a process **lives over time**:

- Dev servers (`npm run dev`, `cargo run`, `flask run`, `vite`) you want up while you keep working
- Watchers (`cargo watch`, `jest --watch`, `tsc -w`, `nodemon`)
- Long builds/tests where you want to do other things and be told when they exit
- Interactive programs that need stdin: REPLs (`python`, `node`, `irb`), `ssh`, `psql`, `gdb`, anything with a prompt
- Anything that emits a TTY-only experience (progress bars, colored output, paging)
- Any command you'd otherwise background with `&` and then poll with `sleep` + re-check — Voxcaster replaces that anti-pattern

Keep using the plain `Bash` tool for **quick, one-shot** commands that return
promptly (`ls`, `git status`, `grep`, a fast build). Voxcaster's value is
persistence and interactivity; don't add its overhead to a command that finishes
in a second.

## The tools

| Tool | Use it for |
|------|-----------|
| `pty_spawn` | Start a process. `command` + `args` array (never a shell string). Returns a `pty_<id>`. |
| `pty_read` | Read the scrollback (`offset`/`limit` by line, `pattern` to glob-filter, `raw` for ANSI, `format:"json"` to parse). |
| `pty_write` | Send stdin to a running session (answer a prompt, type a REPL command — include `\n`). |
| `pty_wait` | Block until the process exits or `timeout_seconds` elapses; returns exit code. The guaranteed completion path. |
| `pty_list` | Enumerate sessions and their status (`running` / `exited` / `killed`). |
| `pty_kill` | Stop a session (`cleanup:true` also frees its buffer). |

Every tool takes an optional `format: "text" | "json"`. Use `json` when you want
to parse fields (exit code, line counts, status) reliably; `text` is the readable
default.

## Core patterns

**Start something and keep working.** Spawn it, then move on. Read its output
later with `pty_read` when you need to check progress.

```
pty_spawn(command="npm", args=["run","dev"], title="dev server")
# ... do other work ...
pty_read(id="pty_…", pattern="*Local:*")   # find the URL it printed
```

**Run a job and learn when it's done — hands-free.** Set `notify_on_exit: true`.
If the session was launched with Claude Code channels enabled (e.g. via the
`verity` launcher), Voxcaster pushes a `<channel source="voxcaster" exit_code=…>`
event into your session **when the process exits**, so you react without polling.
When channels aren't active, this is silently skipped — so for guaranteed
completion, use `pty_wait`.

```
pty_spawn(command="cargo", args=["test"], notify_on_exit=true, title="tests")
# either wait for the <channel> exit event, OR:
pty_wait(id="pty_…", timeout_seconds=300)
```

**Never sleep-and-poll.** If you want to block on completion, call `pty_wait` —
don't spawn then loop `sleep` + `pty_read`. `pty_wait` returns the moment the
process exits (or your timeout hits).

**Interactive input.** Drive a REPL or answer a prompt with `pty_write` (remember
the newline), then `pty_read` the response.

```
pty_spawn(command="python", args=["-i"], title="repl")
pty_write(id="pty_…", data="import sys; print(sys.version)\n")
pty_read(id="pty_…", limit=5)
```

**Find something in noisy output.** `pty_read(pattern="*error*")` glob-filters the
scrollback instead of returning thousands of lines.

## What to know

- **argv only, no shell string.** `pty_spawn` takes a `command` + `args` array.
  To use shell features (pipes, `&&`, redirection) run them *through* a shell:
  `command="bash", args=["-c","make && ./run"]` (or `cmd /C …` on Windows).
- **Command policy.** A built-in floor always blocks catastrophic commands
  (recursive root deletion, disk formatting, pipe-to-shell, fork bombs, etc.). If
  a spawn is denied "by policy", that's the floor or the operator's
  `VOXCASTER_POLICY` — don't try to route around it; tell the user.
- **Session-bound lifetime.** Sessions live as long as the Voxcaster server (the
  current agent session). They're reaped on shutdown — they do **not** survive a
  session restart. Don't assume a `pty_…` id from a previous session still exists;
  `pty_list` to confirm.
- **Line-addressed output.** Reads are by line (`offset`/`limit`), ANSI is
  stripped by default (`raw:true` to keep it).
- **Spawn latency.** On some Windows hosts a PTY spawn can take a couple of
  seconds to initialize — give `pty_wait` a sensible timeout and don't assume an
  instant exit.

## Quick decision rule

If you're about to run something with the Bash tool and you think "this will keep
running" or "I'll need to send it input" or "I want to be told when it finishes"
— stop and use `pty_spawn` instead.
