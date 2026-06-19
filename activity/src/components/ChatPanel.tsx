import { useEffect, useRef, useState } from "react";
import type { ChatMessage } from "../types";
import { sendChat } from "../api";

interface Props {
  messages: ChatMessage[];
  myUserId: string;
}

export function ChatPanel({ messages, myUserId }: Props) {
  const [input, setInput] = useState("");
  const [sending, setSending] = useState(false);
  const bottomRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages.length]);

  const handleSend = async () => {
    const content = input.trim();
    if (!content || sending) return;
    setSending(true);
    setInput("");
    try {
      await sendChat(content);
    } catch {
      setInput(content); // 실패 시 복원
    } finally {
      setSending(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      handleSend();
    }
  };

  return (
    <div style={styles.container}>
      <div style={styles.header}>💬 채팅</div>
      <div style={styles.messageList}>
        {messages.length === 0 && (
          <div style={styles.empty}>메시지가 없습니다</div>
        )}
        {messages.map((msg, i) => {
          const isMe = msg.user_id === myUserId;
          const time = new Date(msg.timestamp_ms).toLocaleTimeString("ko-KR", {
            hour: "2-digit",
            minute: "2-digit",
          });
          return (
            <div key={i} style={{ ...styles.message, ...(isMe ? styles.myMessage : {}) }}>
              <div style={styles.msgMeta}>
                <span style={{ ...styles.username, ...(isMe ? styles.myUsername : {}) }}>
                  {msg.username}
                </span>
                <span style={styles.time}>{time}</span>
              </div>
              <div style={styles.content}>{msg.content}</div>
            </div>
          );
        })}
        <div ref={bottomRef} />
      </div>
      <div style={styles.inputRow}>
        <input
          style={styles.input}
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder="메시지 입력..."
          maxLength={2000}
          disabled={sending}
        />
        <button
          style={{ ...styles.sendBtn, opacity: (!input.trim() || sending) ? 0.5 : 1 }}
          onClick={handleSend}
          disabled={!input.trim() || sending}
        >
          전송
        </button>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: {
    display: "flex",
    flexDirection: "column",
    border: "1px solid rgba(255,255,255,0.1)",
    borderRadius: 10,
    overflow: "hidden",
    maxHeight: 240,
  },
  header: {
    fontSize: 12,
    fontWeight: 600,
    color: "#aaa",
    padding: "6px 10px",
    borderBottom: "1px solid rgba(255,255,255,0.08)",
  },
  messageList: {
    flex: 1,
    overflowY: "auto",
    padding: "6px 10px",
    display: "flex",
    flexDirection: "column",
    gap: 4,
  },
  empty: {
    fontSize: 11,
    color: "#555",
    textAlign: "center",
    padding: "10px 0",
  },
  message: {
    display: "flex",
    flexDirection: "column",
    gap: 2,
  },
  myMessage: {},
  msgMeta: {
    display: "flex",
    alignItems: "center",
    gap: 6,
  },
  username: {
    fontSize: 11,
    fontWeight: 600,
    color: "#90caf9",
  },
  myUsername: {
    color: "#a5d6a7",
  },
  time: {
    fontSize: 10,
    color: "#555",
  },
  content: {
    fontSize: 13,
    color: "#ddd",
    wordBreak: "break-word",
    lineHeight: 1.4,
  },
  inputRow: {
    display: "flex",
    borderTop: "1px solid rgba(255,255,255,0.08)",
    padding: "6px 8px",
    gap: 6,
  },
  input: {
    flex: 1,
    background: "rgba(255,255,255,0.06)",
    border: "1px solid rgba(255,255,255,0.12)",
    borderRadius: 6,
    color: "#fff",
    fontSize: 13,
    padding: "4px 8px",
    outline: "none",
  },
  sendBtn: {
    background: "#5865f2",
    border: "none",
    borderRadius: 6,
    color: "#fff",
    fontSize: 12,
    fontWeight: 600,
    padding: "4px 10px",
    cursor: "pointer",
    whiteSpace: "nowrap",
  },
};
