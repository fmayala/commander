#!/usr/bin/env bash
set -euo pipefail

# Commander Autoresearch Loop
# Each iteration: Claude picks an issue, fixes it, tests, keeps or discards.
# The shell loop restarts Claude for each experiment (fresh context per iteration).

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

cd "$PROJECT_DIR"

# Create autoresearch branch if not already on one
CURRENT_BRANCH=$(git branch --show-current)
if [[ ! "$CURRENT_BRANCH" == autoresearch/* ]]; then
    TAG=$(date +%b%d | tr '[:upper:]' '[:lower:]')
    BRANCH="autoresearch/$TAG"
    echo "Creating branch: $BRANCH"
    git checkout -b "$BRANCH"
else
    echo "Already on autoresearch branch: $CURRENT_BRANCH"
fi

echo ""
echo "=== Commander Autoresearch ==="
echo "Branch: $(git branch --show-current)"
echo ""

ITERATION=0

while true; do
    ITERATION=$((ITERATION + 1))
    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo "  Experiment #$ITERATION — $(date '+%H:%M:%S')"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo ""

    # Run one experiment via Claude
    claude -p \
        --dangerously-skip-permissions \
        --model sonnet \
        --verbose \
        --max-budget-usd 5 \
        "You are the commander autoresearch agent. Read autoresearch/program.md for full instructions.

Your task for this iteration:
1. Read autoresearch/results.tsv to see what's been done
2. Read git log (git log --oneline -20) to see recent changes
3. Run cargo test --workspace and count passing tests
4. Run cargo clippy --workspace and count warnings
5. Pick the HIGHEST PRIORITY unaddressed issue from PRODUCTION_SCOPE.md
6. Implement a focused fix with tests
7. Run cargo test --workspace — verify ALL tests pass
8. Run cargo clippy --workspace — verify no new warnings
9. If tests pass and no regressions: commit and append KEEP to results.tsv
10. If tests fail: git reset --hard HEAD~ and append DISCARD to results.tsv
11. Report what you did

IMPORTANT:
- Check results.tsv and git log FIRST to avoid re-doing work
- One focused fix per iteration — don't try to fix everything
- ALWAYS add tests for your fix
- If a test fails after your change, DISCARD (git reset --hard) — do not try to debug endlessly
- Commit format: fix(ISSUE-ID): brief description
- Log format in results.tsv (tab-separated): commit\ttests_passed\ttests_total\twarnings\tstatus\tdescription"

    EXIT_CODE=$?

    if [ $EXIT_CODE -ne 0 ]; then
        echo ""
        echo "Claude exited with code $EXIT_CODE — pausing 10s before retry"
        sleep 10
    fi

    # Brief pause between iterations
    sleep 2
done
