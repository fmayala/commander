import { useWorkspace } from "../../store/workspace";
import { RepoItem } from "./RepoItem";
import styles from "./Sidebar.module.css";

export function Sidebar() {
  const repos = useWorkspace((s) => s.repos);
  const agentsMap = useWorkspace((s) => s.agents);
  const activeCount = Array.from(agentsMap.values()).filter(
    (a) => a.status === "working" || a.status === "waiting"
  ).length;
  const maxAgents = useWorkspace((s) => s.maxAgents);

  return (
    <div className={styles.sidebar}>
      <div className={styles.header}>Workspace</div>
      <div className={styles.repoList}>
        {repos.map((repo) => (
          <RepoItem key={repo.id} repo={repo} />
        ))}
      </div>
      <div className={styles.footer}>
        <div className={styles.slotUsage}>
          <span className={styles.slotDot} />
          {activeCount}/{maxAgents} agents active
        </div>
      </div>
    </div>
  );
}
