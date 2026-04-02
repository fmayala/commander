# Commander Benchmark Framework — Strategy (Deferred)

> **Status:** Documented for future reference. Not currently being implemented. The autoresearch loop (see `2026-04-02-autoresearch-loop-design.md`) is the active focus.

## Overview

A benchmark framework for validating commander's ability to dispatch agents that produce working, high-quality code. Two fronts:

1. **Real work** — use commander on actual work repos (flares-api, solstice-mono, etc.) to complete real tasks
2. **Code reproduction** — take high-quality open source repos, extract structured tasks, and see if commander's agents can reproduce working code without seeing the source

## Task Suite Format

```yaml
name: "repo-name-feature"
source_repo: "https://github.com/..."
source_commit: "abc123"
extraction_mode: "manual | automated | git_replay"
workspace_setup:
  base_commit: "abc123~5"
  seed_files: []

tasks:
  - id: "task-id"
    title: "Task title"
    description: "..."
    acceptance_criteria: [...]
    files: ["src/**"]
    test_command: "cargo test"
    depends_on: []
    reference:
      commit: "abc123"
      files: ["src/module.rs"]
```

## Three Extraction Modes

- **Manual** — human writes tasks from domain knowledge. Best for personal work repos.
- **Automated** — LLM reads repo, generates task definitions that describe behavior without revealing implementation.
- **Git-replay** — each meaningful commit becomes a task. Agent gets repo at commit N + commit message, must produce commit N+1.

## Scoring (Three Dimensions)

- **Functional (pass/fail)** — compiles, tests pass
- **Structural (0.0–1.0)** — file layout, module boundaries, pattern similarity vs reference. LLM-evaluated.
- **Quality (0.0–1.0)** — idiomatic, clean, maintainable. LLM-evaluated independently of reference.

## Architecture

```
commander-benchmark/
├── extractor/          # Task extraction from source repos
│   ├── manual          # CLI for hand-writing task suites
│   ├── automated       # LLM-powered task generation
│   └── git_replay      # Walk git history → task per commit
├── runner/             # Feed task suites to commander, manage workspaces
├── evaluator/          # Score agent output (functional, structural, quality)
├── reporter/           # Aggregate results per suite
└── suites/             # Stored benchmark task suites
```

## When to Implement

After commander is reliable (autoresearch loop succeeds on critical fixes) and can dispatch real tasks. The benchmark framework becomes the richer fitness function for continued autoresearch iterations, and the validation mechanism for production readiness.
