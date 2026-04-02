import { useState } from "react";
import type { Repo } from "../../types";
import { useWorkspace } from "../../store/workspace";
import styles from "./RepoItem.module.css";

const STATUS_COLORS: Record<string, string> = {
  working: "var(--green)",
  waiting: "var(--yellow)",
  paused: "var(--yellow)",
  completed: "var(--text-dim)",
  failed: "var(--red)",
};

export function RepoItem({ repo }: { repo: Repo }) {
  const agentsMap = useWorkspace((s) => s.agents);
  const select = useWorkspace((s) => s.select);
  const agents = Array.from(agentsMap.values()).filter((a) => a.repoId === repo.id);
  const [expanded, setExpanded] = useState(agents.length > 0);

  const hasActive = agents.some((a) => a.status === "working" || a.status === "waiting");
  const hasFailed = agents.some((a) => a.status === "failed");

  const dotColor = hasFailed
    ? "var(--red)"
    : hasActive
      ? "var(--green)"
      : "var(--text-dim)";

  return (
    <div className={styles.repoItem}>
      <div
        className={styles.repoRow}
        data-active={hasActive}
        onClick={() => {
          setExpanded(!expanded);
          select({ kind: "repo", repoId: repo.id });
        }}
      >
        <span className={styles.statusDot} style={{ background: dotColor }} />
        <span className={styles.repoName}>{repo.name}</span>
        <button
          className={styles.quickAdd}
          onClick={(e) => {
            e.stopPropagation();
          }}
        >
          +
        </button>
      </div>
      {expanded && agents.length > 0 && (
        <div className={styles.agents}>
          {agents.map((agent) => (
            <div
              key={agent.id}
              className={styles.agentRow}
              onClick={() => select({ kind: "agent", agentId: agent.id })}
            >
              <span
                className={styles.agentDot}
                style={{ background: STATUS_COLORS[agent.status] ?? "var(--text-dim)" }}
              />
              {agent.id}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
