import type { Agent, ChatMessage, Repo, Task, TranscriptEntry } from "../types";

let entryCounter = 0;
function entryId(): string {
  return `te-${++entryCounter}`;
}

export const MOCK_REPOS: Repo[] = [
  { id: "repo-1", name: "flares-api", path: "/Users/francisco/Documents/Work/flares-api" },
  { id: "repo-2", name: "solstice-dapp", path: "/Users/francisco/Documents/Copy/solstice-mono/apps/solstice-dapp" },
  { id: "repo-3", name: "worker-pricing", path: "/Users/francisco/Documents/Work/flares-jobs/apps/worker-exponent-pricing" },
  { id: "repo-4", name: "infra-dev", path: "/Users/francisco/Documents/Work/infra-dev-v2-deployment" },
  { id: "repo-5", name: "protocol-mono", path: "/Users/francisco/Documents/Work/protocol-mono" },
];

export const MOCK_TRANSCRIPT: TranscriptEntry[] = [
  {
    id: entryId(),
    kind: "thinking",
    timestamp: Date.now() - 60000,
    content: "I need to read the existing auth middleware to understand the current implementation before making changes.",
  },
  {
    id: entryId(),
    kind: "tool_call",
    timestamp: Date.now() - 55000,
    content: "Reading file to understand current auth implementation",
    toolName: "Read",
    toolInput: "src/middleware/auth.rs",
  },
  {
    id: entryId(),
    kind: "tool_result",
    timestamp: Date.now() - 54000,
    content: "pub struct AuthMiddleware {\n    jwt_secret: String,\n    excluded_paths: Vec<String>,\n}\n\nimpl AuthMiddleware {\n    pub fn new(secret: String) -> Self { ... }\n    pub async fn verify(&self, token: &str) -> Result<Claims> { ... }\n}",
    isError: false,
  },
  {
    id: entryId(),
    kind: "thinking",
    timestamp: Date.now() - 50000,
    content: "The existing middleware has basic JWT verification but lacks rate limiting. I need to add a RateLimiter that tracks requests per IP and per API key.",
  },
  {
    id: entryId(),
    kind: "tool_call",
    timestamp: Date.now() - 45000,
    content: "Writing rate limiting middleware",
    toolName: "Write",
    toolInput: "src/middleware/rate_limit.rs",
  },
  {
    id: entryId(),
    kind: "tool_result",
    timestamp: Date.now() - 44000,
    content: "File written: src/middleware/rate_limit.rs (87 lines)",
    isError: false,
  },
  {
    id: entryId(),
    kind: "text",
    timestamp: Date.now() - 40000,
    content: "I've created the rate limiting middleware. Now I need to integrate it into the router and add tests.",
  },
  {
    id: entryId(),
    kind: "tool_call",
    timestamp: Date.now() - 35000,
    content: "Running test suite to verify",
    toolName: "Bash",
    toolInput: "cargo test -p flares-api",
  },
  {
    id: entryId(),
    kind: "tool_result",
    timestamp: Date.now() - 30000,
    content: "running 12 tests\ntest middleware::rate_limit::tests::test_rate_limit_basic ... ok\ntest middleware::rate_limit::tests::test_rate_limit_per_key ... ok\ntest middleware::rate_limit::tests::test_rate_limit_exceeded ... ok\ntest result: ok. 12 passed; 0 failed",
    isError: false,
  },
];

export const MOCK_AGENTS: Agent[] = [
  {
    id: "agent-07",
    repoId: "repo-1",
    taskId: "task-1",
    status: "working",
    startedAt: Date.now() - 120000,
    transcript: [...MOCK_TRANSCRIPT],
  },
  {
    id: "agent-12",
    repoId: "repo-3",
    taskId: "task-2",
    status: "working",
    startedAt: Date.now() - 60000,
    transcript: [
      {
        id: entryId(),
        kind: "thinking",
        timestamp: Date.now() - 58000,
        content: "Reading the pricing worker test files to understand what's currently tested.",
      },
      {
        id: entryId(),
        kind: "tool_call",
        timestamp: Date.now() - 55000,
        content: "Listing test files",
        toolName: "Bash",
        toolInput: "find src/tasks -name '*test*' -o -name '*spec*'",
      },
      {
        id: entryId(),
        kind: "tool_result",
        timestamp: Date.now() - 54000,
        content: "src/tasks/pricing_test.rs\nsrc/tasks/exponent_test.rs",
        isError: false,
      },
    ],
  },
];

export const MOCK_TASKS: Task[] = [
  {
    id: "task-1",
    repoId: "repo-1",
    title: "Add rate limiting to API endpoints",
    description: "Implement per-IP and per-API-key rate limiting for all public endpoints",
    acceptanceCriteria: [
      "Rate limiter middleware created",
      "Configurable limits per endpoint",
      "429 response with Retry-After header",
      "Tests pass",
    ],
    priority: "P1",
    status: "claimed",
    assignedAgentId: "agent-07",
    dependsOn: [],
    files: ["src/middleware/**", "src/routes/**", "tests/**"],
    attempts: [
      {
        agentId: "agent-07",
        startedAt: Date.now() - 120000,
        endedAt: null,
        outcome: null,
        reason: null,
      },
    ],
  },
  {
    id: "task-2",
    repoId: "repo-3",
    title: "Fix pricing worker test failures",
    description: "Run test suite, identify failing tests, and fix them",
    acceptanceCriteria: ["All tests pass", "No regressions"],
    priority: "P2",
    status: "claimed",
    assignedAgentId: "agent-12",
    dependsOn: [],
    files: ["src/tasks/**"],
    attempts: [
      {
        agentId: "agent-12",
        startedAt: Date.now() - 60000,
        endedAt: null,
        outcome: null,
        reason: null,
      },
    ],
  },
  {
    id: "task-3",
    repoId: "repo-1",
    title: "Add request logging middleware",
    description: "Structured logging for all API requests with trace IDs",
    acceptanceCriteria: ["Structured JSON logs", "Trace ID propagation"],
    priority: "P2",
    status: "pending",
    assignedAgentId: null,
    dependsOn: ["task-1"],
    files: ["src/middleware/**"],
    attempts: [],
  },
];

export const MOCK_CHAT_MESSAGES: ChatMessage[] = [
  {
    id: "msg-1",
    type: "user",
    timestamp: Date.now() - 130000,
    text: "add rate limiting to the flares-api endpoints",
  },
  {
    id: "msg-2",
    type: "commander",
    timestamp: Date.now() - 128000,
    text: "I'll add rate limiting to **flares-api**. Creating task:\n\n- **Task:** Add rate limiting to API endpoints\n- **Criteria:** rate limiter middleware, configurable limits, 429 responses, tests\n- **Files:** `src/middleware/**`, `src/routes/**`, `tests/**`\n\nDispatching now.",
  },
  {
    id: "msg-3",
    type: "dispatch",
    timestamp: Date.now() - 125000,
    text: "agent-07 → flares-api · rate-limiting · running",
    agentId: "agent-07",
    taskId: "task-1",
    repoId: "repo-1",
  },
  {
    id: "msg-4",
    type: "user",
    timestamp: Date.now() - 65000,
    text: "also check the pricing worker tests",
  },
  {
    id: "msg-5",
    type: "commander",
    timestamp: Date.now() - 63000,
    text: "On it. Spawning an agent in **worker-pricing** to run the test suite and fix any failures.",
  },
  {
    id: "msg-6",
    type: "dispatch",
    timestamp: Date.now() - 61000,
    text: "agent-12 → worker-pricing · fix test failures · running",
    agentId: "agent-12",
    taskId: "task-2",
    repoId: "repo-3",
  },
];
