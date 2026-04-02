# Commander Autoresearch Loop — Design Spec

## Overview

An autonomous self-improvement loop for commander, inspired by Karpathy's autoresearch. An LLM agent repeatedly modifies commander's codebase to fix known issues and improve reliability, using test pass rate as the fitness function. The loop runs independently of commander itself — it's a standalone script that optimizes commander's code.

**Goal:** Make commander reliable enough for real task dispatch by autonomously fixing the 5 critical issues and other known problems documented in PRODUCTION_SCOPE.md.

**Approach:** Karpathy-style loop — modify, test, keep-or-discard, repeat. No human in the loop once started.

---

## The Loop

```
┌─────────────────────────────────────────────────────┐
│ 1. Read current state                                │
│    - PRODUCTION_SCOPE.md (known issues)              │
│    - Recent git log (what's been tried)              │
│    - Current test results (baseline)                 │
│    - results.tsv (experiment history)                │
├─────────────────────────────────────────────────────┤
│ 2. Pick an improvement                               │
│    - Choose one focused fix from known issues        │
│    - Or add a new test for an untested path          │
│    - Or improve an existing implementation           │
├─────────────────────────────────────────────────────┤
│ 3. Modify code + commit                              │
│    - Edit source files in crates/                    │
│    - Add or update tests                             │
│    - Commit with descriptive message                 │
├─────────────────────────────────────────────────────┤
│ 4. Run fitness function                              │
│    - cargo test --workspace                          │
│    - cargo clippy --workspace                        │
│    - Count: tests passed, tests added, warnings      │
├─────────────────────────────────────────────────────┤
│ 5. Evaluate                                          │
│    - All existing tests still pass? (no regressions) │
│    - New tests added? (test count increased)         │
│    - No new warnings?                                │
│    - Compiles clean?                                 │
├─────────────────────────────────────────────────────┤
│ 6. Keep or discard                                   │
│    - If improved: keep commit, log to results.tsv    │
│    - If regressed: git reset --hard, log as discard  │
│    - If crash: examine error, log, reset             │
├─────────────────────────────────────────────────────┤
│ 7. Repeat — do not stop, do not ask                  │
└─────────────────────────────────────────────────────┘
```

---

## File Structure

```
autoresearch/
├── program.md              # System prompt for the agent (the "skill")
├── run.sh                  # Main loop script
├── results.tsv             # Experiment log (commit, tests_passed, tests_total, status, description)
└── run.log                 # Current experiment output
```

The loop script and program.md live in `autoresearch/` at the project root. The agent modifies files in `crates/` — the actual commander source.

---

## program.md — Agent Instructions

The program.md is the system prompt given to the agent. It must contain:

### Context
- What commander is (AI agent orchestration kernel)
- Architecture overview (14 crates, supervisor/agent/tool layers)
- Where to find the known issues (PRODUCTION_SCOPE.md — 5 critical, 9 high, 10 medium, 7 low)

### Scope
- **Modify:** Any file in `crates/` 
- **Read-only:** `Cargo.toml`, `commander.toml`, `PRODUCTION_SCOPE.md`
- **Do not modify:** `autoresearch/`, `prototype/`, `docs/`

### Rules
- One focused fix per iteration. Don't try to fix multiple issues at once.
- Always add or update tests for the code you change. Test count must increase or stay the same.
- Follow existing patterns in the codebase. Don't restructure things outside your fix.
- Prefer simple, minimal changes. A 4-line fix that works beats a 40-line refactor.
- If a fix requires understanding code you haven't read, read it first. Don't guess.
- Commit with a message that references the issue ID (e.g., "fix(CRIT-001): worker exit code on failure").

### Priority Order
Work through issues in this order:
1. CRIT-001 through CRIT-005 (critical — task loss, security)
2. HIGH-001 through HIGH-009 (high — reliability, performance)
3. MED-001 through MED-010 (medium — correctness, robustness)
4. LOW-001 through LOW-007 (low — polish, edge cases)

If the current issue is too complex for one iteration, break it into a smaller piece and do that.

### Autonomy
Once the loop begins, do not pause to ask the human. Continue working until manually stopped, even if you exhaust obvious ideas. If stuck on one issue, move to the next. If all known issues are addressed, look for new ones (missing test coverage, edge cases, code quality).

---

## Fitness Function

The fitness function runs after each commit:

```bash
# 1. Compile
cargo build --workspace 2>&1

# 2. Run tests
cargo test --workspace 2>&1

# 3. Lint
cargo clippy --workspace -- -D warnings 2>&1

# 4. Count metrics
#    - tests_passed: number of tests that passed
#    - tests_total: total test count
#    - warnings: number of compiler/clippy warnings
#    - compile_ok: did it compile? (bool)
```

**Keep criteria (ALL must be true):**
- `compile_ok == true`
- `tests_passed == tests_total` (no failures)
- `tests_total >= previous_tests_total` (no test deletions)
- `warnings <= previous_warnings` (no new warnings)

**Bonus signals (logged but don't gate keep/discard):**
- Tests added (tests_total increased)
- Warnings decreased
- Lines of code changed (prefer smaller diffs)

---

## results.tsv Format

Tab-separated, one row per experiment:

```
commit	tests_passed	tests_total	warnings	status	description
abc1234	96	96	2	keep	fix(CRIT-001): worker exit code on failure
def5678	95	97	2	discard	fix(CRIT-002): claim atomicity — broke existing test
ghi9012	97	97	1	keep	fix(CRIT-002): claim atomicity with timeout recovery
```

Status values: `keep`, `discard`, `crash`

---

## run.sh — Loop Script

The agent handles the full loop internally — there is no outer orchestration script. Like Karpathy's approach, the agent itself:
1. Reads the current state (program.md, results.tsv, git log, test results)
2. Picks an issue and implements a fix
3. Commits
4. Runs the fitness function (`cargo test`, `cargo clippy`)
5. Evaluates the results
6. Keeps the commit or resets
7. Logs to results.tsv
8. Continues to the next iteration

`run.sh` is just a launcher that starts the agent session. All decision-making, git operations, and evaluation happen inside the agent's session.

**Implementation options for the agent invocation:**
- **Claude Code CLI** — `claude --print -p "read program.md and run the next experiment"` with tool access to Read, Write, Edit, Bash
- **Direct API** — script calls Anthropic API with program.md as system prompt and tool definitions for file I/O
- **Commander itself** — once commander is functional, it could run the loop (bootstrapping)

Start with Claude Code CLI — it's the simplest and already has tool access.

---

## Branch Strategy

The loop runs on a dedicated branch:

```bash
git checkout -b autoresearch/apr02
```

Each kept commit advances the branch. Discarded commits are reset. The human can review the branch at any time, cherry-pick good fixes to main, or merge the whole branch when satisfied.

---

## Stopping Conditions

The loop runs until manually stopped. Natural stopping points:
- All 5 critical issues resolved
- All 96+ tests pass with new coverage for previously untested paths
- Agent reports no more obvious improvements to make

The human reviews results.tsv periodically and merges good work to main.

---

## Future Evolution

Once commander is reliable (autoresearch loop succeeds on critical fixes):
1. **Richer fitness function** — add integration tests that actually dispatch a task end-to-end
2. **Benchmark suite** — use the deferred benchmark framework (see `2026-04-02-benchmark-framework-strategy.md`) as the fitness function
3. **Self-hosting** — run the autoresearch loop through commander itself (commander dispatches agents that improve commander)
4. **Multi-agent** — multiple agents working on different issues in parallel, each on their own branch

---

## Out of Scope

- The benchmark framework (documented separately, deferred)
- Visual UI for monitoring the loop (use results.tsv + git log)
- Multi-agent coordination (single agent for now)
- Automated merging to main (human reviews and merges)
