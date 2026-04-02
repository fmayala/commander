import { useWorkspace } from "../../store/workspace";
import { RepoItem } from "./RepoItem";
import styles from "./Sidebar.module.css";

export function Sidebar() {
  // rerender-derived-state: subscribe to derived counts, not the full agents Map
  const repos = useWorkspace((s) => s.repos);
  const activeCount = useWorkspace((s) => s.activeAgentCount());
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
