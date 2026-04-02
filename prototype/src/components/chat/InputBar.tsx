import { useCallback, useMemo, useRef, useState } from "react";
import { useChat } from "../../store/chat";
import { useWorkspace } from "../../store/workspace";
import { MentionDropdown } from "./MentionDropdown";
import type { ChatMessage, MentionOption } from "../../types";
import styles from "./InputBar.module.css";

export function InputBar() {
  // rerender-defer-reads: only subscribe to addMessage, not the full chat state
  const addMessage = useChat((s) => s.addMessage);
  const repos = useWorkspace((s) => s.repos);
  const agents = useWorkspace((s) => s.agents);
  const tasks = useWorkspace((s) => s.tasks);
  const [value, setValue] = useState("");
  const [mentionQuery, setMentionQuery] = useState<string | null>(null);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const textareaRef = useRef<HTMLTextAreaElement>(null);

  const mentionOptions = useMemo((): MentionOption[] => {
    if (mentionQuery === null) return [];
    const q = mentionQuery.toLowerCase();

    const repoOptions: MentionOption[] = repos
      .filter((r) => r.name.toLowerCase().includes(q))
      .map((r) => ({ kind: "repo", id: r.id, label: r.name }));

    const agentOptions: MentionOption[] = Array.from(agents.values())
      .filter((a) => a.id.toLowerCase().includes(q))
      .map((a) => ({
        kind: "agent",
        id: a.id,
        label: a.id,
        secondary: repos.find((r) => r.id === a.repoId)?.name,
      }));

    const taskOptions: MentionOption[] = Array.from(tasks.values())
      .filter((t) => t.id.toLowerCase().includes(q) || t.title.toLowerCase().includes(q))
      .map((t) => ({
        kind: "task",
        id: t.id,
        label: t.id,
        secondary: t.title,
      }));

    return [...repoOptions, ...agentOptions, ...taskOptions];
  }, [mentionQuery, repos, agents, tasks]);

  const insertMention = useCallback(
    (option: MentionOption) => {
      const atIdx = value.lastIndexOf("@");
      if (atIdx === -1) return;
      const before = value.slice(0, atIdx);
      const newValue = `${before}@${option.label} `;
      setValue(newValue);
      setMentionQuery(null);
      setSelectedIndex(0);
      textareaRef.current?.focus();
    },
    [value]
  );

  const handleChange = useCallback((e: React.ChangeEvent<HTMLTextAreaElement>) => {
    const v = e.target.value;
    setValue(v);

    const atIdx = v.lastIndexOf("@");
    if (atIdx !== -1) {
      const charBefore = v[atIdx - 1];
      if (atIdx === 0 || charBefore === " " || charBefore === "\n") {
        const query = v.slice(atIdx + 1);
        if (!query.includes(" ")) {
          setMentionQuery(query);
          setSelectedIndex(0);
          return;
        }
      }
    }
    setMentionQuery(null);
  }, []);

  const send = useCallback(() => {
    const trimmed = value.trim();
    if (!trimmed) return;

    const msg: ChatMessage = {
      id: `msg-${Date.now()}`,
      type: "user",
      timestamp: Date.now(),
      text: trimmed,
    };
    addMessage(msg);
    setValue("");
    setMentionQuery(null);

    // Mock commander response
    setTimeout(() => {
      const reply: ChatMessage = {
        id: `msg-${Date.now()}`,
        type: "commander",
        timestamp: Date.now(),
        text: `Received: "${trimmed}". (Mock response — dispatch simulation not wired yet.)`,
      };
      addMessage(reply);
    }, 500);
  }, [value, addMessage]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (mentionQuery !== null && mentionOptions.length > 0) {
        if (e.key === "ArrowDown") {
          e.preventDefault();
          // rerender-functional-setstate: use functional form for derived state
          setSelectedIndex((i) => Math.min(i + 1, mentionOptions.length - 1));
          return;
        }
        if (e.key === "ArrowUp") {
          e.preventDefault();
          setSelectedIndex((i) => Math.max(i - 1, 0));
          return;
        }
        if (e.key === "Tab" || e.key === "Enter") {
          e.preventDefault();
          const option = mentionOptions[selectedIndex];
          if (option) insertMention(option);
          return;
        }
        if (e.key === "Escape") {
          e.preventDefault();
          setMentionQuery(null);
          return;
        }
      }

      if (e.key === "Enter" && !e.shiftKey) {
        e.preventDefault();
        send();
      }
    },
    [mentionQuery, mentionOptions, selectedIndex, insertMention, send]
  );

  return (
    <div className={styles.inputBar}>
      {mentionQuery !== null && (
        <MentionDropdown
          options={mentionOptions}
          selectedIndex={selectedIndex}
          onSelect={insertMention}
        />
      )}
      <div className={styles.inputRow}>
        <textarea
          ref={textareaRef}
          className={styles.textarea}
          value={value}
          onChange={handleChange}
          onKeyDown={handleKeyDown}
          placeholder="Ask commander anything... (@ to mention)"
          rows={1}
        />
        <button className={styles.sendBtn} onClick={send}>
          Send
        </button>
      </div>
      <div className={styles.footer}>
        <span className={styles.model}>claude-sonnet-4.6 ▾</span>
        <span>Enter to send · Shift+Enter for newline</span>
      </div>
    </div>
  );
}
