# Claude Code Offload — Architecture Plan

## Summary

A `/offload` skill for Claude Code that hands off the current session to a warm AgentCore runtime in AWS, where it runs to completion while the laptop is closed. Continuous shadowing keeps the remote peer pre-synced so kick is ~1s. Take-back restores full conversation state (via `claude --resume`) for local continuation.

Target user: AWS shops running long-form coding tasks they want to continue after closing their laptop.

## Goals

- `/offload` slash command in CC that ships the current session to remote execution.
- `/offload list`, `/offload pull <id>`, `/offload status <id>` for managing in-flight work.
- ~1s offload latency (no cold start).
- Full conversation state transfer (not just task description) using `claude --resume`.
- AWS-native end-to-end: AgentCore runtime, Bedrock-Claude model, IAM/CodeCommit, S3, DynamoDB.
- Optimize for capability and UX, not cost. Always-warm, always-on, no idle suspension.

## Non-goals (v1)

- Multi-agent coordination / mesh of N peers. Architecture extends naturally to N peers; not built now.
- Vendor-agnostic destinations. AgentCore only.
- Strong consistency or CRDT merge. Last-writer-wins is sufficient.
- Pair programming (concurrent active peers).
- Replacing CC's internal context manager. The skill is a participant, not a replacement.

## Requirements

- **No cold start.** Rules out "warm runtime only" approaches. Pushes the design to continuous shadow.
- **AWS-native.** Destination is AgentCore + Bedrock + S3 + DDB + CodeCommit/GitHub.
- **Compatible with CC.** Uses CC's native skill system, hooks, headless mode, and `--resume`.

## Architecture

### Components

**Local laptop**
- Claude Code (user's normal session).
- `/offload` skill at `~/.claude/skills/offload/` (or repo-local `.claude/skills/offload/`).
- Shadow daemon — small Go or Python binary, watches project files + CC session transcript, runs as launchd/systemd.

**AWS**
- **API Gateway (WebSocket)** — low-latency control plane.
- **Lambda** — WebSocket handler; routes events to DDB and AgentCore.
- **DynamoDB** — single source of truth per session.
- **S3** — file blobs and session.jsonl chunks.
- **AgentCore runtime** — always-warm session per user, holds the remote shadow peer.
- **IAM** — credentials and scoping; Bedrock-Claude via `CLAUDE_CODE_USE_BEDROCK=1`.
- **CodeCommit or existing GitHub** — git transport for code.

### Data flow

```
[Laptop]                            [AWS]
─────────                           ─────
CC session ──┐
             │
shadow       ├─► API Gateway WS ──► Lambda ──► DynamoDB (state)
daemon ──────┘                                    │
   │                                              │
   └────────► S3 (file blobs ──────────────► AgentCore session
              + session.jsonl                 (always-warm)
              chunks)                         /tmp/work mirror
                                              claude --resume on promote
```

### State model

**DynamoDB table** `claude-offload-state`:
- PK: `userId#sessionId`
- Fields:
  - `active_peer`: `"local"` | `"remote"` | `"none"` — single source of truth, prevents split-brain.
  - `branch`: git branch for this offload.
  - `task`: task description from user.
  - `status`: `"shadowing"` | `"running"` | `"complete"` | `"error"`.
  - `cursor`: last-applied event index (for ordering).
  - `started_at`, `updated_at`, `completed_at`.

**S3 layout**:
- `s3://bucket/{userId}/{sessionId}/files/…` — mirrored repo (excluding ignore list).
- `s3://bucket/{userId}/{sessionId}/session.jsonl` — current transcript.
- `s3://bucket/{userId}/{sessionId}/events/{cursor}.json` — append-only event log.

### Promote flow (`/offload`)

`/offload` relies on CC's mid-task input handling. Two behaviors are possible (and the mechanic is in flux — see Risks):

- **Queued (today's actual behavior):** typing while CC is busy silently queues; the skill fires at the next turn boundary, after in-flight tool calls finish.
- **Interrupting (documented behavior, may become default):** typing interrupts the current turn; the skill fires immediately.

Either is acceptable for offload. The promote sequence below works in both modes — only the timing of step 1 differs.

1. Skill writes "promote" event over WebSocket; Lambda sets `active_peer = remote` in DDB.
2. AgentCore peer (already mirroring) confirms its cursor matches DDB. If it lags, it drains pending S3 events first.
3. Peer runs `claude --resume <id> -p "<continuation>" --dangerously-skip-permissions --output-format json`. Continuation is the user's `/offload <message>` argument, or a default "continue where you left off."
4. Output streams back via WebSocket; user sees progress in the skill (observe mode) or can detach.
5. On completion: peer pushes final state to S3, sets `active_peer = none`, status `complete`.

### Take-back flow (`/offload pull <id>`)

Two cases.

**Completed run.**
1. Skill checks DDB; status is `complete`.
2. Fetches latest session.jsonl + repo state from S3.
3. Restores `~/.claude/projects/<hash>/sessions/<id>.jsonl` locally.
4. Sets `active_peer = local`.
5. User runs `claude --resume <id>` to continue locally with full conversation history.

**Running session (graceful take-back).**
1. User confirms the take-back prompt.
2. Skill writes "yield" event; remote peer finishes its current tool call, persists state, exits at the next turn boundary.
3. Once remote yields, take-back proceeds as the completed-run path.

Both promote and take-back honor turn boundaries — neither side interrupts the other mid-tool-call.

### CC skill structure

```
~/.claude/skills/offload/
├── SKILL.md              # frontmatter + instructions
├── lib/
│   ├── setup.sh          # first-run linking
│   ├── kick.sh           # offload current session
│   ├── list.sh           # list offloaded sessions
│   ├── pull.sh           # pull a finished session back
│   └── status.sh         # check status of a session
├── runtime/
│   ├── agent.py          # AgentCore entrypoint
│   ├── Dockerfile
│   └── requirements.txt
└── daemon/
    ├── shadow.go         # local shadow daemon
    └── ignore.txt        # default exclude list
```

**SKILL.md frontmatter**:
```yaml
---
name: offload
description: Offload current Claude Code session to a remote AgentCore runtime. Subcommands: list, pull, status, setup.
allowed-tools: Bash, Read, Write, Edit
user-invocable: true
---
```

## User experience

### Mid-task handoff (the core mechanic)

`/offload` is designed to be invokable while CC is actively working, not just at idle. CC's current behavior is to **silently queue** typed input during a busy turn and run it after the current turn completes — so `/offload` typed mid-task fires at the next turn boundary. (Interrupt-on-Enter is the documented behavior; the discrepancy is tracked upstream and may flip in either direction. See Risks.)

Why turn boundaries matter: that's when `session.jsonl` is consistent — no half-completed tool calls, no partial assistant turns. The remote can `--resume` cleanly from there. If CC switches to interrupt-by-default, the skill captures partial state instead; the remote still resumes correctly because the JSONL is still self-consistent up to the interruption point.

Sequence when invoked mid-task (queue mode, today):

```
[CC] Editing src/gateway.rs...
> /offload                                     ← user types mid-task
[CC] Queued. Will hand off at next turn boundary.
[CC] ...finishes current edit, runs cargo check...
[CC] Promoting to remote (140ms).
[CC] Local session is now passive. Remote is active.
     Status: /offload status abc123… or detach (Ctrl+C).
```

The shadow daemon has already streamed the latest `session.jsonl` and project state to S3, so the remote peer's mirror is current. Promote is just a DDB flag flip + a `claude --resume` invocation on the remote — no state transfer at kick time.

### Continuation prompt

`/offload` (no args) → remote receives `"Continue the current task; resume where you were interrupted."`

`/offload <message>` → the message is the continuation prompt to remote. Use this to add context as you hand off ("`/offload also update the changelog`").

### Detach vs. observe

After promote, two modes:

**Detach** — close the terminal, sleep the laptop. Remote keeps working. Notification when done. `/offload pull <id>` from any future CC session restores state. This is the close-the-laptop use case.

**Observe** — remote streams output back over WebSocket; local CC displays remote's progress. Input is locked while `active_peer = remote` (you can't type into a session you don't own). Useful for monitoring; you can detach at any point with Ctrl+C.

### First run

```
> /offload
First-time setup. I'll link this machine to AWS AgentCore.
- AWS region? [us-east-1]
- Git remote? [auto-detected: origin → CodeCommit/example]
- Building runtime image and registering AgentCore runtime...
- Creating DynamoDB table claude-offload-state...
- Starting shadow daemon...
- Warming first AgentCore session... (~30s, one-time)
Done. Configuration saved to ~/.claude/offload/config.json.
```

### Listing

```
> /offload list
ID        STATUS    BRANCH              STARTED   TASK
abc123…   running   offload/171...      3m ago    "Add tests for gateway"
def456…   complete  offload/170...      2h ago    "Refactor MCP edge transport"
```

### Pulling a finished session

```
> /offload pull abc123…
Fetching state from S3...
Diff: 14 files changed, +320 -45.
Session restored. Run `claude --resume abc123…` to continue locally.
```

### Pulling a still-running session (graceful take-back)

```
> /offload pull abc123…
Remote is still running. Take back anyway? This will:
  - Signal remote to checkpoint at next turn boundary and exit.
  - Pull current state to local.
  - Make local active again.
[y/N]
```

Symmetric with promote: the remote also yields only at a turn boundary, so neither direction interrupts in-flight work.

### Status

```
> /offload status abc123…
Branch: offload/1714234567-a1b2c3d4
Active peer: remote
Status: running (12m elapsed)
Last event: tool_call edit_file src/gateway.rs (3s ago)
Cost so far: $0.42
```

## Cost (informational)

Cost is not a design constraint for v1. Listed here only so order-of-magnitude is documented.

AgentCore Runtime: $0.0895/vCPU-hr + $0.00945/GB-hr, per-second granularity. An always-warm shadow at ~5% CPU and 1GB peak runs roughly $10/user/month. API Gateway WS, DynamoDB, S3, Lambda, ECR are pennies. No auto-suspend; warm always.

If cost ever becomes a constraint, the natural levers are: auto-suspend after extended idle, smaller memory footprint, or moving the shadow to spot-priced Fargate. None of these are in scope now.

## Build phases

Each phase is independently shippable.

### Phase 0 — validate primitives (~3 hours, no AWS infra)

Three tests determine whether the architecture is sound. Run before building anything else.

1. **`claude --resume` cross-machine.** Copy a session.jsonl from one project checkout to a sibling checkout (different cwd), placed at the corresponding `~/.claude/projects/<hash>/sessions/` path. Run `claude --resume <id> -p "continue what you were doing"` from the sibling. Confirm it loads history and produces useful work.
2. **`claude -p` quality.** Run `claude -p "<real ticket from this repo>" --dangerously-skip-permissions --output-format json`. Evaluate output: did it understand the task, make reasonable edits, and report a useful summary?
3. **CC inside a Linux container.** `docker run` an Ubuntu image with Node 20 + claude-code installed. Mount the repo, run `claude -p "..."` from inside. Confirm the tool stack (bash, git, npm) works.

**Decision gate:** if any fail, revise the design before continuing. Specifically, if test 1 fails because of project-hash routing, the runtime needs hash-rewriting logic. If test 2 fails, conversation transfer alone isn't enough — design needs an iteration loop or richer task framing.

### Phase 1 — static offload (~1 day)

- Skill scaffolding (`SKILL.md`, `kick.sh`, `setup.sh`).
- AgentCore agent.py that clones repo + runs `claude --resume`.
- DynamoDB state table.
- S3 bucket for session.jsonl transfer.
- Cold-start path only — no shadow yet.
- Validates end-to-end: skill → AgentCore → execution → result back.

### Phase 2 — warm runtime (~½ day)

- Persistent `runtime-session-id` per user.
- 14-min keepalive ping (cron or Lambda + EventBridge).
- Eager warming during `setup` so the first real offload is warm.
- Cold start drops to 5–10s for state transfer; runtime itself is warm.

### Phase 3 — continuous shadow (~3–4 days)

- Local shadow daemon: file watcher + JSONL tail + S3 sync.
- API Gateway WebSocket + Lambda for control plane.
- AgentCore peer continuously applies S3 events to mirror.
- `active_peer` coordination via DDB.
- Promote becomes ~1s.

### Phase 4 — polish (~2 days)

- Take-back state restoration with conflict detection.
- `list`, `status`, `pull` subcommands.
- Streaming output to local terminal (optional).
- Observability: CloudWatch dashboards, error reporting.

## Open decisions

| Decision | Options | Notes |
|---|---|---|
| Shadow daemon language | Go vs. Python | Go for single-binary distribution; Python for parity with agent.py. |
| Git transport | CodeCommit vs. existing GitHub | CodeCommit for AWS-native pitch; GitHub for friction-free if user already has it. |
| File watcher | chokidar (Node) vs. fsnotify (Go) vs. watchman | Pick with daemon language. |
| Conflict policy | Freeze local during active remote (v1) vs. CRDT merge (v2) | Freeze is simpler; CRDT is later. |
| Bedrock model selection | Claude latest vs. user-pinned | Default latest; allow override. |

## Risks

- **CC `-p` quality.** May not be useful enough for non-trivial tasks. Mitigation: test in Phase 0; if it fails, design an iteration loop where the agent self-evaluates and continues.
- **Project-hash routing.** Session paths may not transfer cleanly cross-machine. Mitigation: compute hash inside agent.py; if format changes, abstract behind a helper.
- **AgentCore preview APIs.** Will change during build. Mitigation: pin SDK version; budget for one migration during dev.
- **Vendor preemption.** Anthropic may ship native local→web push, undercutting the casual-user pitch. Mitigation: lean into the AWS-native enterprise angle that vendor solutions can't easily match.
- **File-watcher noise.** Aggressive ignore list required (`node_modules`, `target`, `.venv`, `.git`, build artifacts). Mitigation: ship sane defaults, allow per-project overrides via `.offloadignore`.
- **Offline laptop.** Daemon must queue + checkpoint, replay on reconnect. Mitigation: standard append-log pattern with cursor.
- **AgentCore 8h session ceiling.** Long-running shadows must chain sessions. Mitigation: spin up new session before expiry, transfer mirror, retire old. ~30s bridging window mid-day.

## Validation tests (Phase 0 in detail)

### Test 1 — `claude --resume` cross-machine

```bash
# In project-a/
SESSION_ID=$(uuidgen)
claude -p "Make a small edit to README.md" --session-id "$SESSION_ID"
SESSION_FILE=$(find ~/.claude/projects -name "${SESSION_ID}.jsonl" | head -1)

# Copy to project-b/ (a different checkout of the same repo or a sibling project)
cd ../project-b
TARGET_DIR=$(claude --print-project-dir 2>/dev/null || \
  echo "~/.claude/projects/$(echo "$PWD" | sha256sum | head -c 32)/sessions")
mkdir -p "$TARGET_DIR"
cp "$SESSION_FILE" "$TARGET_DIR/${SESSION_ID}.jsonl"

# Try to resume
claude --resume "$SESSION_ID" -p "What did you just do? Continue."
# Expected: references the prior edit and continues sensibly.
```

### Test 2 — `claude -p` on real work

Pick a real ticket from this repo (e.g., a small bug or feature in `src/gateway.rs`). Then:

```bash
claude -p "$(cat <<'EOF'
Read src/gateway.rs. Identify any error handling that swallows errors silently.
For each, propose a fix as a unified diff.
EOF
)" --dangerously-skip-permissions --output-format json > result.json

jq '.result' result.json
jq '.total_cost_usd' result.json
```

Evaluate: did it find real issues? Are the diffs sensible? Cost reasonable?

### Test 3 — CC inside Linux container

```dockerfile
# Dockerfile.test
FROM ubuntu:24.04
RUN apt-get update && apt-get install -y curl git ca-certificates
RUN curl -fsSL https://deb.nodesource.com/setup_20.x | bash - && \
    apt-get install -y nodejs
RUN npm install -g @anthropic-ai/claude-code
WORKDIR /work
```

```bash
docker build -t cc-test -f Dockerfile.test .
docker run --rm -it \
  -v "$PWD:/work" \
  -e ANTHROPIC_API_KEY \
  cc-test \
  claude -p "List the top-level files in this repo and describe the project."
```

Expected: works, lists files, summarizes. If it fails, debug the package or auth; the AgentCore version inherits whatever works here.

## What to build first

Run Phase 0 (the three tests) before any AWS work. They take an afternoon and gate the entire architecture. After they pass, Phase 1 is mechanical.
