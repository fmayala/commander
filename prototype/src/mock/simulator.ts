import { useWorkspace } from "../store/workspace";
import { useChat } from "../store/chat";
import type { TranscriptEntry, ChatMessage } from "../types";

const SIMULATED_ENTRIES: Omit<TranscriptEntry, "id" | "timestamp">[] = [
  { kind: "thinking", content: "Let me check if there are any edge cases I'm missing in the implementation." },
  { kind: "tool_call", content: "Reading related file", toolName: "Read", toolInput: "src/config/limits.rs" },
  { kind: "tool_result", content: "pub const DEFAULT_RATE_LIMIT: u32 = 100;\npub const WINDOW_SECONDS: u64 = 60;", isError: false },
  { kind: "thinking", content: "Good — the config already defines rate limit constants. I'll wire these into the middleware." },
  { kind: "tool_call", content: "Updating middleware to use config", toolName: "Edit", toolInput: "src/middleware/rate_limit.rs:24-35" },
  { kind: "tool_result", content: "File updated: src/middleware/rate_limit.rs", isError: false },
  { kind: "text", content: "Rate limiting is now using the centralized config. Running tests to confirm." },
  { kind: "tool_call", content: "Running tests", toolName: "Bash", toolInput: "cargo test -p flares-api -- --nocapture" },
  { kind: "tool_result", content: "running 15 tests\ntest result: ok. 15 passed; 0 failed; 0 ignored", isError: false },
  { kind: "text", content: "All tests pass. The rate limiting implementation is complete. Task done." },
];

let entryCounter = 1000;
let simIndex = 0;

function nextEntry(): TranscriptEntry {
  const template = SIMULATED_ENTRIES[simIndex % SIMULATED_ENTRIES.length]!;
  simIndex++;
  return {
    ...template,
    id: `sim-${++entryCounter}`,
    timestamp: Date.now(),
  };
}

let intervalId: ReturnType<typeof setInterval> | null = null;

export function startSimulator(): void {
  if (intervalId) return;

  intervalId = setInterval(() => {
    const agents = Array.from(useWorkspace.getState().agents.values());
    const working = agents.filter((a) => a.status === "working");

    if (working.length === 0) return;

    const agent = working[Math.floor(Math.random() * working.length)]!;
    const entry = nextEntry();

    useWorkspace.getState().appendTranscript(agent.id, entry);

    if (simIndex > 0 && simIndex % SIMULATED_ENTRIES.length === 0) {
      useWorkspace.getState().updateAgent(agent.id, { status: "completed" });
      useWorkspace.getState().updateTask(agent.taskId, { status: "complete" });

      const completionMsg: ChatMessage = {
        id: `msg-sim-${entryCounter}`,
        type: "completion",
        timestamp: Date.now(),
        text: `${agent.id} · completed ✓ · task finished`,
        agentId: agent.id,
        taskId: agent.taskId,
        repoId: agent.repoId,
      };
      useChat.getState().addMessage(completionMsg);
    }
  }, 3000);
}

export function stopSimulator(): void {
  if (intervalId) {
    clearInterval(intervalId);
    intervalId = null;
  }
}
