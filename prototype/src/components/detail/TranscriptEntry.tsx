import { useState } from "react";
import type { TranscriptEntry as TEntry } from "../../types";
import styles from "./TranscriptEntry.module.css";

export function TranscriptEntry({ entry }: { entry: TEntry }) {
  const [collapsed, setCollapsed] = useState(true);

  switch (entry.kind) {
    case "thinking":
      return (
        <div
          className={`${styles.entry} ${styles.thinking}`}
          data-collapsed={collapsed}
          onClick={() => setCollapsed(!collapsed)}
        >
          <span className={styles.thinkingLabel}>⌬ thinking</span>
          {entry.content}
        </div>
      );

    case "tool_call":
      return (
        <div className={styles.toolCall}>
          <span className={styles.toolName}>{entry.toolName}</span>
          <span className={styles.toolInput}>{entry.toolInput}</span>
        </div>
      );

    case "tool_result":
      return (
        <div
          className={styles.toolResult}
          data-error={entry.isError}
          data-collapsed={collapsed}
          onClick={() => setCollapsed(!collapsed)}
        >
          {entry.content}
        </div>
      );

    case "text":
      return (
        <div className={`${styles.entry} ${styles.text}`}>{entry.content}</div>
      );

    case "error":
      return (
        <div className={`${styles.entry} ${styles.error}`}>{entry.content}</div>
      );

    default:
      return null;
  }
}
