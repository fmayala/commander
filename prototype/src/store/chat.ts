import { create } from "zustand";
import type { ChatMessage } from "../types";

interface ChatState {
  messages: ChatMessage[];
  inputValue: string;
  mentionQuery: string | null;

  addMessage: (message: ChatMessage) => void;
  setInput: (value: string) => void;
  setMentionQuery: (query: string | null) => void;
  clearMessages: () => void;
}

export const useChat = create<ChatState>((set) => ({
  messages: [],
  inputValue: "",
  mentionQuery: null,

  addMessage: (message) =>
    set((s) => ({ messages: [...s.messages, message] })),

  setInput: (value) => set({ inputValue: value }),

  setMentionQuery: (query) => set({ mentionQuery: query }),

  clearMessages: () => set({ messages: [] }),
}));
