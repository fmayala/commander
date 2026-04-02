import type { ChatMessage } from "../../types";
import { useWorkspace } from "../../store/workspace";
import styles from "./MessageItem.module.css";

export function MessageItem({ message }: { message: ChatMessage }) {
  // rerender-defer-reads: only subscribe to select, not the full workspace state
  const select = useWorkspace((s) => s.select);

  switch (message.type) {
    case "user":
      return (
        <div className={`${styles.message} ${styles.user}`}>
          {message.text}
        </div>
      );

    case "commander":
      return (
        <div className={`${styles.message} ${styles.commander}`}>
          {message.text}
        </div>
      );

    case "dispatch":
      return (
        <div
          className={`${styles.message} ${styles.dispatch}`}
          onClick={() => {
            if (message.agentId) select({ kind: "agent", agentId: message.agentId });
          }}
        >
          <span>●</span>
          <span>{message.text}</span>
        </div>
      );

    case "completion":
      return (
        <div
          className={`${styles.message} ${styles.completion}`}
          onClick={() => {
            if (message.agentId) select({ kind: "agent", agentId: message.agentId });
          }}
        >
          <span>✓</span>
          <span>{message.text}</span>
        </div>
      );

    case "failure":
      return (
        <div className={styles.message}>
          <div
            className={styles.failure}
            onClick={() => {
              if (message.agentId) select({ kind: "agent", agentId: message.agentId });
            }}
          >
            <span>✗</span>
            <span>{message.text}</span>
          </div>
          <div className={styles.actions}>
            <button
              className={styles.actionBtn}
              onClick={() => {
                if (message.agentId) select({ kind: "agent", agentId: message.agentId });
              }}
            >
              View session →
            </button>
            <button className={styles.actionBtn}>Retry</button>
            <button className={styles.actionBtn}>Dismiss</button>
          </div>
        </div>
      );

    default:
      return null;
  }
}
