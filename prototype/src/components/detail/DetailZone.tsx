import { useWorkspace } from "../../store/workspace";
import { AgentSession } from "./AgentSession";
import { TaskView } from "./TaskView";
import { WorkspaceDashboard } from "./WorkspaceDashboard";
import styles from "./DetailZone.module.css";

const STATUS_COLORS: Record<string, string> = {
  working: "var(--green)",
  waiting: "var(--yellow)",
  paused: "var(--yellow)",
  completed: "var(--text-dim)",
  failed: "var(--red)",
};

export function DetailZone() {
  const selection = useWorkspace((s) => s.selection);
  const agents = useWorkspace((s) => s.agents);
  const select = useWorkspace((s) => s.select);

  const agentTabs = Array.from(agents.values()).filter(
    (a) =>
      a.status === "working" ||
      a.status === "waiting" ||
      (selection.kind === "agent" && selection.agentId === a.id)
  );

  if (selection.kind === "none") {
    return (
      <div className={styles.zone}>
        <div className={styles.content}>
          <WorkspaceDashboard />
        </div>
      </div>
    );
  }

  return (
    <div className={styles.zone}>
      {agentTabs.length > 0 && (
        <div className={styles.tabs}>
          {agentTabs.map((agent) => (
            <button
              key={agent.id}
              className={styles.tab}
              data-active={selection.kind === "agent" && selection.agentId === agent.id}
              onClick={() => select({ kind: "agent", agentId: agent.id })}
            >
              <span
                className={styles.tabDot}
                style={{ background: STATUS_COLORS[agent.status] ?? "var(--text-dim)" }}
              />
              {agent.id}
              <button
                className={styles.tabClose}
                onClick={(e) => {
                  e.stopPropagation();
                  select({ kind: "none" });
                }}
              >
                ×
              </button>
            </button>
          ))}
        </div>
      )}
      <div className={styles.content}>
        {selection.kind === "agent" && <AgentSession agentId={selection.agentId} />}
        {selection.kind === "task" && <TaskView taskId={selection.taskId} />}
        {selection.kind === "repo" && <WorkspaceDashboard />}
      </div>
    </div>
  );
}
