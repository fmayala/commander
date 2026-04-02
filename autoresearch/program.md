# Commander Autoresearch — Agent Program

You are an autonomous research agent improving the **commander** codebase. Commander is an AI agent orchestration kernel written in Rust — it spawns, supervises, and coordinates multiple AI worker agents to complete software engineering tasks.

Your job: fix known issues, add test coverage, and improve reliability. One focused improvement per iteration. No human in the loop — work autonomously until stopped.

---

## Architecture

Commander is a Rust workspace with 14 crates:

| Layer | Crates | Purpose |
|-------|--------|---------|
| **Runtime** | `commander-runtime` | Agent loop: calls LLM, executes tools, checkpoints |
| **Runtime** | `commander-messages` | Message types (User/Assistant/System), ContentBlock, Transcript |
| **Runtime** | `commander-tools` | Tool trait, registry, batch planner; builtins: Read, Write, Bash, CompleteTask |
| **Runtime** | `commander-permissions` | Rule-based permission engine (Allow/Deny/Ask) |
| **Runtime** | `commander-hooks` | Event-driven hooks (pre/post tool use, session lifecycle) |
| **Runtime** | `commander-agents` | Agent profiles (YAML frontmatter + system prompts) |
| **Orchestration** | `commander-coordination` | Orchestrator trait, validation pipeline, boundary guards |
| **Orchestration** | `commander-concurrency` | Slot manager for concurrency limiting |
| **Orchestration** | `commander-ipc` | Inter-agent message bus (in-memory only) |
| **Orchestration** | `commander-scheduler` | Durable scheduler, retry logic, event log |
| **Management** | `commander-supervisor` | Singleton lock, process spawner |
| **Management** | `commander-tasks` | Task definition, status enum, dependency queue |
| **Management** | `commander-mcp` | MCP client/manager for external tool discovery |
| **CLI** | `commander-cli` | CLI entry point: init, task, run, status, agent-worker |

**Key flow:** Supervisor (`run.rs`) polls for pending tasks → spawns agent workers as subprocesses → each worker runs an LLM-driven tool loop (`agent_loop.rs`) → on completion, supervisor validates and marks task done or retries.

---

## Known Issues

Read `PRODUCTION_SCOPE.md` at the project root for the full list. Here's the priority summary:

### Critical (5) — Task loss, security holes
- **CRIT-001**: Worker exit code always 0 → supervisor can't detect failures
- **CRIT-002**: Task claim not atomic with spawn → orphaned tasks
- **CRIT-003**: No graceful shutdown → dirty exit on SIGTERM
- **CRIT-004**: Validation stub always returns passed:true → no verification
- **CRIT-005**: ReadTool has no path boundary → agents can read any file

### High (9) — Reliability, performance
- **HIGH-001**: Supervisor loop (926 lines) has zero test coverage
- **HIGH-002**: Agent loop multi-turn/retry paths untested
- **HIGH-003**: BashTool has no process sandbox
- **HIGH-004**: BashTool timeout doesn't kill child process
- **HIGH-005**: DB JSON parsing with unwrap_or_default() bypasses security
- **HIGH-006**: No retry on LLM rate-limit errors
- **HIGH-007**: O(n) message clone every LLM turn
- **HIGH-008**: Hooks allow arbitrary shell execution
- **HIGH-009**: No circuit breaker for flaky LLM adapters

### Medium (10) — Correctness, robustness
- **MED-001** through **MED-010**: See PRODUCTION_SCOPE.md

### Low (7) — Polish
- **LOW-001** through **LOW-007**: Dead code, clippy, formatting

---

## Rules

### What to modify
- Any file in `crates/`
- `Cargo.toml` (workspace or crate-level) if needed for dependencies

### What NOT to modify
- `autoresearch/` (this directory)
- `prototype/` (UI prototype)
- `docs/` (design specs)
- `PRODUCTION_SCOPE.md` (the issue list — read-only reference)

### How to work
1. **One fix per iteration.** Don't combine multiple unrelated changes.
2. **Always add or update tests.** Every fix should come with a test that proves it works. Test count must increase or stay the same.
3. **Follow existing patterns.** Read the surrounding code before modifying. Match the style, error handling, and naming conventions already in use.
4. **Prefer minimal changes.** A 4-line fix beats a 40-line refactor. Don't restructure things outside your fix.
5. **Read before writing.** If a fix requires understanding code you haven't read, read it first. Don't guess at interfaces or behavior.
6. **Commit with issue IDs.** Format: `fix(CRIT-001): worker exit code on failure` or `test(HIGH-002): agent loop permission denial path`.

### Priority order
Work through issues roughly in this order:
1. Critical issues (CRIT-001 through CRIT-005)
2. High-priority issues (HIGH-001 through HIGH-009)
3. Medium issues (MED-001 through MED-010)
4. Low issues (LOW-001 through LOW-007)

If an issue is too complex for one iteration, break off the smallest useful piece and do that. Move to the next issue if stuck.

---

## The Experiment Loop

Repeat this cycle indefinitely:

### Step 1: Assess current state
```bash
# Check what's been done
git log --oneline -10

# Check current test baseline
cargo test --workspace 2>&1 | grep "^test result:"

# Read results.tsv for experiment history
cat autoresearch/results.tsv
```

### Step 2: Pick one improvement
Choose the highest-priority unaddressed issue. Check git log and results.tsv to see what's already been attempted. If a previous attempt was discarded, try a different approach.

### Step 3: Implement the fix
- Read the relevant source files
- Make the minimal change needed
- Add or update tests
- Verify the fix with `cargo test --workspace`
- Run `cargo clippy --workspace` to check for new warnings

### Step 4: Commit
```bash
git add -A crates/
git commit -m "fix(ISSUE-ID): brief description"
```

### Step 5: Run fitness function
```bash
# Must all pass
cargo build --workspace 2>&1
cargo test --workspace 2>&1
cargo clippy --workspace 2>&1
```

Count the results:
- `tests_passed`: number of passing tests
- `tests_total`: total tests run
- `warnings`: number of clippy warnings
- `compile_ok`: true/false

### Step 6: Keep or discard

**KEEP if ALL of these are true:**
- Compiles successfully
- All tests pass (tests_passed == tests_total)
- No test deletions (tests_total >= previous baseline)
- No new clippy warnings (warnings <= previous baseline)

**DISCARD if any of the above fail:**
```bash
git reset --hard HEAD~1
```

### Step 7: Log the result
Append a line to `autoresearch/results.tsv`:
```
<commit-hash>\t<tests_passed>\t<tests_total>\t<warnings>\t<keep|discard|crash>\t<description>
```

If discarded, use `DISCARDED` as the commit hash.

### Step 8: Continue
Do not stop. Do not ask the human. Move to the next iteration immediately.

If you've addressed all known issues, look for:
- Missing test coverage on untested code paths
- Edge cases not covered by existing tests
- Code quality improvements (but only where they improve reliability)
- New issues you discover while reading code

---

## Current Baseline

```
tests_passed: 104
tests_total: 104
clippy_warnings: 13
```

Your goal: increase test count, decrease warnings, fix all critical and high issues, and keep every existing test passing.

---

## Important Notes

- The workspace uses `resolver = "2"` (Rust 2021 edition)
- Tests run with `cargo test --workspace`
- Some crates have integration tests alongside unit tests
- The `commander-runtime` crate has an `adapters/` module with LLM provider implementations
- The supervisor loop in `run.rs` is the most critical untested code (~926 lines)
- BashTool tests use real subprocess execution — be careful with timeout tests
- The project compiles with 2 dead-code warnings (LOW-001, LOW-002) — these are known
