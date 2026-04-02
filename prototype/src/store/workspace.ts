import { create } from "zustand";
import type { Agent, DetailSelection, Repo, Task } from "../types";

interface WorkspaceState {
  repos: Repo[];
  agents: Map<string, Agent>;
  tasks: Map<string, Task>;
  selection: DetailSelection;
  maxAgents: number;

  setRepos: (repos: Repo[]) => void;
  addAgent: (agent: Agent) => void;
  updateAgent: (agentId: string, patch: Partial<Agent>) => void;
  appendTranscript: (agentId: string, entry: Agent["transcript"][number]) => void;
  removeAgent: (agentId: string) => void;
  addTask: (task: Task) => void;
  updateTask: (taskId: string, patch: Partial<Task>) => void;
  select: (selection: DetailSelection) => void;

  agentsByRepo: (repoId: string) => Agent[];
  tasksByRepo: (repoId: string) => Task[];
  activeAgentCount: () => number;
}

export const useWorkspace = create<WorkspaceState>((set, get) => ({
  repos: [],
  agents: new Map(),
  tasks: new Map(),
  selection: { kind: "none" },
  maxAgents: 5,

  setRepos: (repos) => set({ repos }),

  addAgent: (agent) =>
    set((s) => {
      const agents = new Map(s.agents);
      agents.set(agent.id, agent);
      return { agents };
    }),

  updateAgent: (agentId, patch) =>
    set((s) => {
      const agents = new Map(s.agents);
      const existing = agents.get(agentId);
      if (existing) agents.set(agentId, { ...existing, ...patch });
      return { agents };
    }),

  appendTranscript: (agentId, entry) =>
    set((s) => {
      const agents = new Map(s.agents);
      const existing = agents.get(agentId);
      if (existing) {
        agents.set(agentId, {
          ...existing,
          transcript: [...existing.transcript, entry],
        });
      }
      return { agents };
    }),

  removeAgent: (agentId) =>
    set((s) => {
      const agents = new Map(s.agents);
      agents.delete(agentId);
      return { agents };
    }),

  addTask: (task) =>
    set((s) => {
      const tasks = new Map(s.tasks);
      tasks.set(task.id, task);
      return { tasks };
    }),

  updateTask: (taskId, patch) =>
    set((s) => {
      const tasks = new Map(s.tasks);
      const existing = tasks.get(taskId);
      if (existing) tasks.set(taskId, { ...existing, ...patch });
      return { tasks };
    }),

  select: (selection) => set({ selection }),

  agentsByRepo: (repoId) =>
    Array.from(get().agents.values()).filter((a) => a.repoId === repoId),

  tasksByRepo: (repoId) =>
    Array.from(get().tasks.values()).filter((t) => t.repoId === repoId),

  activeAgentCount: () =>
    Array.from(get().agents.values()).filter(
      (a) => a.status === "working" || a.status === "waiting"
    ).length,
}));
