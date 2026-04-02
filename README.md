# Commander

A reusable AI agent orchestration platform. Dispatch, supervise, and optimize teams of LLM-powered workers from a single control plane — across any project.

## Why Commander

Working with AI coding agents today means managing multiple terminals, re-writing prompts, and manually coordinating sessions. Commander replaces that with centralized dispatch: you define tasks, and the supervisor handles the rest — spawning agents, monitoring progress, recovering from failures, and validating output.

But the real value is what accumulates over time:

- **Agent profiles are project artifacts, not throwaway prompts.** You define worker configurations — model, system prompt, tools, permissions, filesystem scope — once, and reuse them. "The migration agent" or "the audit agent" becomes a thing you dispatch, not a prompt you re-write every session.

- **Agents get better at your systems.** Commander's autoresearch loop can optimize worker skills, agent configs, and system prompts autonomously. The agents improve at working on *your* codebase without manual tuning.

- **Composable agent abstractions.** Define reusable agents with explicit context and specific configs, then compose them into workflows. Different tasks get routed to the right agent with the right model, the right tools, and the right constraints.

- **Provider-agnostic.** Route different tasks to different LLM providers based on cost, capability, or availability. The supervisor abstracts over the provider layer.

## How It Works

```
┌─────────────────────────────────────────────────┐
│                  Supervisor                      │
│  ┌───────────┐  ┌──────────┐  ┌──────────────┐  │
│  │ Scheduler  │  │ Slot Mgr │  │  Validator   │  │
│  └─────┬─────┘  └────┬─────┘  └──────┬───────┘  │
│        │              │               │          │
│  ┌─────▼──────────────▼───────────────▼───────┐  │
│  │              SQLite Task Queue              │  │
│  └─────┬──────────────┬───────────────┬───────┘  │
│        │              │               │          │
│  ┌─────▼─────┐  ┌─────▼─────┐  ┌─────▼─────┐   │
│  │  Agent 1  │  │  Agent 2  │  │  Agent N  │   │
│  │ (subprocess│  │ (subprocess│  │ (subprocess│   │
│  │  + LLM)   │  │  + LLM)   │  │  + LLM)   │   │
│  └───────────┘  └───────────┘  └───────────┘   │
└─────────────────────────────────────────────────┘
```

The supervisor manages a durable task queue backed by SQLite. Each agent runs as an independent OS process with its own LLM conversation loop, filesystem sandbox, and tool access. The supervisor handles:

- **Concurrency** — Up to N agents in parallel, configurable per-project or globally.
- **Crash recovery** — Orphaned tasks are reclaimed on startup. Heartbeats detect stalled agents. Configurable restart limits.
- **Validation** — Multi-step verification (boundary checks, test execution) before accepting agent output. Failed validation triggers retry cycles.
- **Graceful lifecycle** — `SIGTERM`/`SIGINT` drains active agents before shutdown. No orphaned work.

## Architecture

Rust workspace, 14 crates across four layers:

**Runtime** — The LLM execution engine. Agent loop with checkpointing, circuit breaker, rate-limit retry. Built-in tools (`Read`, `Write`, `Bash`, `CompleteTask`). Agent profiles parsed from YAML frontmatter. Permission engine and hook system.

> `commander-runtime` · `commander-messages` · `commander-tools` · `commander-permissions` · `commander-hooks` · `commander-agents`

**Orchestration** — Task coordination. Validation pipeline, path boundary guards, concurrency slot manager, inter-agent message bus, durable scheduler with retry logic.

> `commander-coordination` · `commander-concurrency` · `commander-ipc` · `commander-scheduler`

**Management** — Process supervision. Singleton lock, process spawner, task state machine (`Pending` → `Claimed` → `Complete`/`Failed`), dependency queue.

> `commander-supervisor` · `commander-tasks`

**CLI & Control** — User-facing interface. Subcommands for project init, task management, supervisor control. MCP client for external tool discovery.

> `commander-cli` · `commander-mcp`

### LLM Providers

Multiple backends via a common `LlmAdapter` trait:

- **Anthropic** (Claude) — default
- **OpenAI** (GPT)
- **OpenRouter** — proxy to many providers
- **Codex** — custom endpoints with JWT validation

## Getting Started

### Prerequisites

- Rust toolchain (edition 2021)
- An API key for at least one supported LLM provider

### Build

```sh
cargo build --workspace
```

### Initialize a Project

```sh
commander init
```

Creates a `.commander/` directory with the SQLite database, default agent profiles, and runtime state.

### Configure

Edit `commander.toml` in your project root:

```toml
[project]
name = "my-project"

[runtime]
provider = "anthropic"           # anthropic | openai | openrouter | codex
default_model = "claude-sonnet-4-6"
max_output_tokens = 16384

[supervisor]
max_agents = 5                   # max concurrent agent subprocesses
tick_interval_ms = 2000          # supervisor poll interval
nudge_after_ms = 120000          # nudge stalled agents after 2min
restart_after_ms = 300000        # restart unresponsive agents after 5min
max_restarts = 2                 # max restart attempts per task

[validation]
# test_command = "cargo test"    # optional: run tests before accepting results
max_fix_cycles = 3               # retry cycles if validation fails
```

### Define Agent Profiles

Agent profiles live in `.commander/profiles/` as Markdown files with YAML frontmatter:

```markdown
---
model: claude-sonnet-4-6
permission_mode: auto
max_turns: 50
timeout: 30m
---

You are a software engineer. Complete the assigned task by reading
the relevant code, making changes, and verifying your work compiles
and passes tests.
```

Different profiles for different jobs — a cautious auditor, an aggressive refactorer, a test writer — each with tuned prompts, models, and constraints.

### Dispatch Work

```sh
# Implementation task (produces code changes)
commander task add "Refactor the auth module" --kind implement --priority P1

# Exploration task (produces a report)
commander task add "Audit error handling patterns" --kind explore --priority P2
```

### Run the Supervisor

```sh
commander run
```

Claims pending tasks, spawns agent workers, monitors heartbeats, validates output. `Ctrl+C` for graceful shutdown.

### Check Status

```sh
commander task list
commander task status <task-id>
commander status
```

## Autoresearch

Commander includes an autonomous research loop (`autoresearch/`) that uses the system to improve itself. Given a list of known issues, autoresearch:

1. Picks the highest-priority unresolved issue
2. Spawns an agent to implement a fix
3. Runs the test suite to validate
4. Commits the fix or discards the attempt
5. Logs the result and moves to the next issue

This same pattern applies to optimizing agent profiles and system prompts — the system tunes its own workers over time.

## Runtime State

All runtime state lives in `.commander/` (gitignored by default):

```
.commander/
├── db.sqlite          # Task queue and agent run history
├── profiles/          # Agent profile definitions
├── agents/            # Per-agent config files
├── results/           # Worker result files (JSON)
├── checkpoints/       # Message history checkpoints
├── heartbeats/        # Heartbeat timestamp files
├── logs/              # Agent stderr logs
└── baselines/         # Baseline file snapshots
```

## Testing

```sh
cargo test --workspace
cargo clippy --workspace
```

## Roadmap

- **Core kernel** — Supervisor, task queue, agent loop, crash recovery, validation *(done)*
- **Autoresearch** — Self-improving agent configs and system prompts *(active)*
- **UI** — Clean interface for dispatching and monitoring agent workers *(next)*
- **Composable agents** — Reusable agent abstractions with bundled context and configs

## Project Status

**Alpha** — Core architecture is implemented and functional. A production-readiness audit identified 31 issues; the majority have been resolved through autoresearch. See [PRODUCTION_SCOPE.md](PRODUCTION_SCOPE.md) for details.

## License

MIT
