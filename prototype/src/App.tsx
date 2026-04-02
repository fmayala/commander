import { useEffect } from "react";
import { useWorkspace } from "./store/workspace";
import { useChat } from "./store/chat";
import { MOCK_REPOS, MOCK_AGENTS, MOCK_TASKS, MOCK_CHAT_MESSAGES } from "./mock/data";
import { startSimulator } from "./mock/simulator";
import { Sidebar } from "./components/sidebar/Sidebar";
import { Chat } from "./components/chat/Chat";
import { DetailZone } from "./components/detail/DetailZone";
import styles from "./App.module.css";

export function App() {
  // rerender-derived-state: subscribe to the derived boolean, not the full selection object
  const detailOpen = useWorkspace((s) => s.selection.kind !== "none");

  useEffect(() => {
    useWorkspace.getState().setRepos(MOCK_REPOS);
    for (const agent of MOCK_AGENTS) {
      useWorkspace.getState().addAgent(agent);
    }
    for (const task of MOCK_TASKS) {
      useWorkspace.getState().addTask(task);
    }
    for (const msg of MOCK_CHAT_MESSAGES) {
      useChat.getState().addMessage(msg);
    }
    startSimulator();
  }, []);

  return (
    <div className={styles.layout} data-detail-open={detailOpen}>
      <div className={styles.sidebar}>
        <Sidebar />
      </div>
      <div className={styles.chat}>
        <Chat />
      </div>
      <div className={detailOpen ? styles.detail : styles.detailHidden}>
        <DetailZone />
      </div>
    </div>
  );
}
