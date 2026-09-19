import { create } from "zustand";
import type { ActionProposal, AssistantEvent, AssistantSource, DraftProposal } from "@/types/assistant";

export const ASSISTANT_OPEN_KEY = "assistant-open";

export interface AssistantTurn {
  id: string;
  question: string;
  answer: string;
  status: string | null;
  sources: AssistantSource[];
  drafts: DraftProposal[];
  actions: ActionProposal[];
  error: { message: string; retryable: boolean } | null;
  done: boolean;
  context: { folder: string; uid: number } | null;
}

interface AssistantState {
  open: boolean;
  turns: AssistantTurn[];
  /** Whether a turn is in flight. Lives here (not component state) so it
   * survives the panel unmounting mid-stream — closing the panel shouldn't
   * silently kill an answer the user can come back to. */
  busy: boolean;
  abortController: AbortController | null;
  setOpen: (open: boolean) => void;
  toggle: () => void;
  startTurn: (question: string, context: AssistantTurn["context"]) => string;
  applyEvent: (id: string, e: AssistantEvent) => void;
  failTurn: (id: string, message: string, retryable: boolean) => void;
  clear: () => void;
  /** Claims the in-flight slot. Returns null if a turn is already running. */
  beginRequest: () => AbortController | null;
  /** Releases the in-flight slot once a turn settles. */
  endRequest: () => void;
  /** Aborts the in-flight request, if any. */
  stopRequest: () => void;
}

function readOpen(): boolean {
  try {
    return typeof localStorage !== "undefined" && localStorage.getItem(ASSISTANT_OPEN_KEY) === "1";
  } catch {
    return false;
  }
}

function writeOpen(open: boolean) {
  try {
    localStorage.setItem(ASSISTANT_OPEN_KEY, open ? "1" : "0");
  } catch {
    // storage blocked: the panel just forgets its state
  }
}

function fold(t: AssistantTurn, e: AssistantEvent): AssistantTurn {
  switch (e.type) {
    case "status":
      return { ...t, status: e.text };
    case "text":
      return { ...t, status: null, answer: t.answer + e.delta };
    case "sources":
      return { ...t, sources: e.sources };
    case "draft":
      return { ...t, drafts: [...t.drafts, e.draft] };
    case "action":
      return { ...t, actions: [...t.actions, e.action] };
    case "error":
      return { ...t, status: null, error: { message: e.message, retryable: e.retryable }, done: true };
    case "done":
      return { ...t, status: null, done: true };
  }
}

export const useAssistantStore = create<AssistantState>((set, get) => ({
  open: readOpen(),
  turns: [],
  busy: false,
  abortController: null,
  setOpen: (open) => {
    writeOpen(open);
    set({ open });
  },
  toggle: () =>
    set((s) => {
      writeOpen(!s.open);
      return { open: !s.open };
    }),
  startTurn: (question, context) => {
    const id = crypto.randomUUID();
    set((s) => ({
      turns: [
        ...s.turns,
        { id, question, answer: "", status: "Thinking…", sources: [], drafts: [], actions: [], error: null, done: false, context },
      ],
    }));
    return id;
  },
  applyEvent: (id, e) => set((s) => ({ turns: s.turns.map((t) => (t.id === id ? fold(t, e) : t)) })),
  failTurn: (id, message, retryable) =>
    set((s) => ({ turns: s.turns.map((t) => (t.id === id ? fold(t, { type: "error", message, retryable }) : t)) })),
  clear: () => set({ turns: [] }),
  beginRequest: () => {
    if (get().busy) return null;
    const controller = new AbortController();
    set({ busy: true, abortController: controller });
    return controller;
  },
  endRequest: () => set({ busy: false, abortController: null }),
  stopRequest: () => get().abortController?.abort(),
}));
