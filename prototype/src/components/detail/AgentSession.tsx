import { useEffect, useRef, useState } from "react";
import { useWorkspace } from "../../store/workspace";
import { TranscriptEntry } from "./TranscriptEntry";
import styles from "./AgentSession.module.css";

const STATUS_COLORS: Record<string, string> = {
  working: "var(--green)",
  waiting: "var(--yellow)",
  paused: "var(--yellow)",
  completed: "var(--text-dim)",
  failed: "var(--red)",
};

function formatElapsed(startedAt: number): string {
  const seconds = Math.floor((Date.now() - startedAt) / 1000);
  const m = Math.floor(seconds / 60);
  const s = seconds % 60;
  return `${m}m ${s}s`;
}

export function AgentSession({ agentId }: { agentId: string }) {
  const agent = useWorkspace((s) => s.agents.get(agentId));
  const repos = useWorkspace((s) => s.repos);
  const updateAgent = useWorkspace((s) => s.updateAgent);
  const bottomRef = useRef<HTMLDivElement>(null);
  const [injectValue, setInjectValue] = useState("");
  const [elapsed, setElapsed] = useState("");

  useEffect(() => {
    if (!agent) return;
    setElapsed(formatElapsed(agent.startedAt));
    const interval = setInterval(() => {
      setElapsed(formatElapsed(agent.startedAt));
    }, 1000);
    return () => clearInterval(interval);
  }, [agent?.startedAt]);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [agent?.transcript.length]);

  if (!agent) {
    return <div style={{ padding: 16, color: "var(--text-dim)" }}>Agent not found</div>;
  }

  const repo = repos.find((r) => r.id === agent.repoId);

  const handleInject = () => {
    const trimmed = injectValue.trim();
    if (!trimmed) return;
    useWorkspace.getState().appendTranscript(agentId, {
      id: `inject-${Date.now()}`,
      kind: "text",
      timestamp: Date.now(),
      content: `[injected] ${trimmed}`,
    });
    setInjectValue("");
  };

  return (
    <div className={styles.session}>
      <div className={styles.header}>
        <span
          className={styles.statusDot}
          style={{ background: STATUS_COLORS[agent.status] ?? "var(--text-dim)" }}
        />
        <span className={styles.agentName}>{agent.id}</span>
        {repo && <span className={styles.repoBadge}>{repo.name}</span>}
        <span className={styles.elapsed}>{elapsed}</span>
        <div className={styles.spacer} />
      </div>
      <div className={styles.transcript}>
        {agent.transcript.map((entry) => (
          <TranscriptEntry key={entry.id} entry={entry} />
        ))}
        <div ref={bottomRef} />
      </div>
      <div className={styles.controls}>
        <button
          className={styles.controlBtn}
          onClick={() =>
            updateAgent(agentId, {
              status: agent.status === "paused" ? "working" : "paused",
            })
          }
        >
          {agent.status === "paused" ? "Resume" : "Pause"}
        </button>
        <input
          className={styles.injectInput}
          value={injectValue}
          onChange={(e) => setInjectValue(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") handleInject();
          }}
          placeholder="Inject message into agent session..."
        />
        <button
          className={styles.controlBtnDanger}
          onClick={() => updateAgent(agentId, { status: "failed" })}
        >
          Cancel
        </button>
      </div>
    </div>
  );
}
