import { useWorkspace } from "../../store/workspace";
import styles from "./WorkspaceDashboard.module.css";

export function WorkspaceDashboard() {
  const agents = useWorkspace((s) => s.agents);
  const tasks = useWorkspace((s) => s.tasks);
  const maxAgents = useWorkspace((s) => s.maxAgents);

  const agentList = Array.from(agents.values());
  const taskList = Array.from(tasks.values());

  const activeAgents = agentList.filter((a) => a.status === "working" || a.status === "waiting").length;
  const pendingTasks = taskList.filter((t) => t.status === "pending" || t.status === "blocked").length;
  const completedTasks = taskList.filter((t) => t.status === "complete").length;

  return (
    <div className={styles.dashboard}>
      <div className={styles.title}>Workspace Overview</div>
      <div className={styles.stats}>
        <div className={styles.stat}>
          <div className={styles.statValue}>
            {activeAgents}/{maxAgents}
          </div>
          <div className={styles.statLabel}>Agents Active</div>
        </div>
        <div className={styles.stat}>
          <div className={styles.statValue}>{pendingTasks}</div>
          <div className={styles.statLabel}>Tasks Queued</div>
        </div>
        <div className={styles.stat}>
          <div className={styles.statValue}>{completedTasks}</div>
          <div className={styles.statLabel}>Completed</div>
        </div>
      </div>
      <div className={styles.recentLabel}>Recent Activity</div>
      <div className={styles.emptyState}>
        Activity feed will appear here as agents complete tasks.
      </div>
    </div>
  );
}
