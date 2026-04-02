import { useEffect, useRef } from "react";
import { useChat } from "../../store/chat";
import { MessageItem } from "./MessageItem";
import { InputBar } from "./InputBar";
import styles from "./Chat.module.css";

export function Chat() {
  const messages = useChat((s) => s.messages);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages.length]);

  return (
    <div className={styles.chat}>
      <div className={styles.messages}>
        {messages.map((msg) => (
          <MessageItem key={msg.id} message={msg} />
        ))}
        <div ref={bottomRef} />
      </div>
      <InputBar />
    </div>
  );
}
