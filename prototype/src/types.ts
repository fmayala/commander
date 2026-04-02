// === Workspace ===

export interface Repo {
  id: string;
  name: string;
  path: string;
}

export type AgentStatus = "working" | "waiting" | "paused" | "completed" | "failed";

export interface Agent {
  id: string;
  repoId: string;
  taskId: string;
  status: AgentStatus;
  startedAt: number;
  transcript: TranscriptEntry[];
}

export type TranscriptEntryKind = "thinking" | "tool_call" | "tool_result" | "text" | "error";

export interface TranscriptEntry {
  id: string;
  kind: TranscriptEntryKind;
  timestamp: number;
  content: string;
  toolName?: string;
  toolInput?: string;
  isError?: boolean;
}

// === Tasks ===

export type TaskStatus =
  | "pending"
  | "claimed"
  | "blocked"
  | "complete"
  | "failed"
  | "retrying"
  | "escalated";

export type Priority = "P0" | "P1" | "P2" | "P3";

export interface Task {
  id: string;
  repoId: string;
  title: string;
  description: string;
  acceptanceCriteria: string[];
  priority: Priority;
  status: TaskStatus;
  assignedAgentId: string | null;
  dependsOn: string[];
  files: string[];
  attempts: TaskAttempt[];
}

export interface TaskAttempt {
  agentId: string;
  startedAt: number;
  endedAt: number | null;
  outcome: "passed" | "failed" | null;
  reason: string | null;
}

// === Chat ===

export type ChatMessageType =
  | "user"
  | "commander"
  | "dispatch"
  | "completion"
  | "failure";

export interface ChatMessage {
  id: string;
  type: ChatMessageType;
  timestamp: number;
  text: string;
  agentId?: string;
  taskId?: string;
  repoId?: string;
  error?: string;
}

// === Mentions ===

export type MentionKind = "repo" | "agent" | "task" | "file";

export interface MentionOption {
  kind: MentionKind;
  id: string;
  label: string;
  secondary?: string;
}

// === Selection ===

export type DetailSelection =
  | { kind: "none" }
  | { kind: "agent"; agentId: string }
  | { kind: "task"; taskId: string }
  | { kind: "repo"; repoId: string };
