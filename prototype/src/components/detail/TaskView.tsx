import { useWorkspace } from "../../store/workspace";
import styles from "./TaskView.module.css";

const STATUS_STYLES: Record<string, { bg: string; color: string }> = {
  pending: { bg: "rgba(163, 163, 163, 0.1)", color: "var(--text-secondary)" },
  claimed: { bg: "rgba(59, 130, 246, 0.1)", color: "var(--accent)" },
  blocked: { bg: "rgba(234, 179, 8, 0.1)", color: "var(--yellow)" },
  complete: { bg: "rgba(34, 197, 94, 0.1)", color: "var(--green)" },
  failed: { bg: "rgba(239, 68, 68, 0.1)", color: "var(--red)" },
  retrying: { bg: "rgba(234, 179, 8, 0.1)", color: "var(--yellow)" },
  escalated: { bg: "rgba(239, 68, 68, 0.1)", color: "var(--red)" },
};

export function TaskView({ taskId }: { taskId: string }) {
  const task = useWorkspace((s) => s.tasks.get(taskId));
  const select = useWorkspace((s) => s.select);

  if (!task) {
    return <div style={{ padding: 16, color: "var(--text-dim)" }}>Task not found</div>;
  }

  const statusStyle = STATUS_STYLES[task.status] ?? STATUS_STYLES.pending!;

  return (
    <div className={styles.taskView}>
      <div className={styles.header}>
        <span className={styles.taskId}>{task.id}</span>
        <span
          className={styles.statusBadge}
          style={{ background: statusStyle.bg, color: statusStyle.color }}
        >
          {task.status}
        </span>
        <span className={styles.priorityBadge}>{task.priority}</span>
      </div>

      <div className={styles.title}>{task.title}</div>
      <div className={styles.description}>{task.description}</div>

      {task.assignedAgentId && (
        <div className={styles.section}>
          <div className={styles.sectionLabel}>Assigned Agent</div>
          <span
            className={styles.agentLink}
            onClick={() => select({ kind: "agent", agentId: task.assignedAgentId! })}
          >
            {task.assignedAgentId}
          </span>
        </div>
      )}

      <div className={styles.section}>
        <div className={styles.sectionLabel}>Acceptance Criteria</div>
        <ul className={styles.criteriaList}>
          {task.acceptanceCriteria.map((c, i) => (
            <li key={i} className={styles.criteriaItem}>
              <span className={styles.checkbox} />
              {c}
            </li>
          ))}
        </ul>
      </div>

      {task.files.length > 0 && (
        <div className={styles.section}>
          <div className={styles.sectionLabel}>File Boundaries</div>
          <ul className={styles.fileList}>
            {task.files.map((f, i) => (
              <li key={i} className={styles.fileItem}>
                {f}
              </li>
            ))}
          </ul>
        </div>
      )}

      {task.attempts.length > 0 && (
        <div className={styles.section}>
          <div className={styles.sectionLabel}>Attempts</div>
          <div className={styles.attemptList}>
            {task.attempts.map((attempt, i) => (
              <div key={i} className={styles.attempt}>
                <span>#{i + 1}</span>
                <span
                  className={styles.agentLink}
                  onClick={() => select({ kind: "agent", agentId: attempt.agentId })}
                >
                  {attempt.agentId}
                </span>
                <span
                  className={styles.attemptOutcome}
                  style={{
                    color: attempt.outcome === "passed" ? "var(--green)" : attempt.outcome === "failed" ? "var(--red)" : "var(--text-dim)",
                  }}
                >
                  {attempt.outcome ?? "in progress"}
                </span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
