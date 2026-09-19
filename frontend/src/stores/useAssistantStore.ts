import { create } from "zustand";
import type { ActionProposal, AssistantEvent, AssistantSource, DraftProposal } from "@/types/assistant";
import { useAuthStore } from "@/stores/useAuthStore";
import { useUiStore } from "@/stores/useUiStore";

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
  /** The active account when this turn was asked. A turn's action/draft
   * cards must refuse to act once this no longer matches the active
   * account: otherwise switching mailboxes lets a stale card confirm
   * against the wrong mailbox's folders/uids. */
  accountId: string | null;
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
  /** Aborts any in-flight request and drops all turns. Called on account
   * switch and on logout so one mailbox's conversation, drafts and action
   * cards never linger over another. */
  reset: () => void;
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
      const next = !s.open;
      writeOpen(next);
      // The ✦ entry points (nav rail, ⌘J, mobile tab) only toggle this
      // panel; the panel itself only renders in mail view. Opening it must
      // also switch to mail view, or it does nothing outside mail. Closing
      // leaves the view as it is.
      if (next) useUiStore.getState().setViewMode("mail");
      return { open: next };
    }),
  startTurn: (question, context) => {
    const id = crypto.randomUUID();
    const accountId = useAuthStore.getState().activeAccountId;
    set((s) => ({
      turns: [
        ...s.turns,
        { id, question, answer: "", status: "Thinking…", sources: [], drafts: [], actions: [], error: null, done: false, context, accountId },
      ],
    }));
    return id;
  },
  applyEvent: (id, e) => set((s) => ({ turns: s.turns.map((t) => (t.id === id ? fold(t, e) : t)) })),
  failTurn: (id, message, retryable) =>
    set((s) => ({ turns: s.turns.map((t) => (t.id === id ? fold(t, { type: "error", message, retryable }) : t)) })),
  clear: () => get().reset(),
  beginRequest: () => {
    if (get().busy) return null;
    const controller = new AbortController();
    set({ busy: true, abortController: controller });
    return controller;
  },
  endRequest: () => set({ busy: false, abortController: null }),
  stopRequest: () => get().abortController?.abort(),
  reset: () => {
    get().abortController?.abort();
    set({ turns: [], busy: false, abortController: null });
  },
}));

// Account switch and logout both end up changing the active account id (see
// AccountSwitcher and NavRail); reset here so a stale conversation, an
// in-flight stream, or a stamped draft/action card never survives across
// mailboxes. Keyed on the id rather than called from each call site, so no
// future account-switching path can forget it.
let lastActiveAccountId = useAuthStore.getState().activeAccountId;
useAuthStore.subscribe((state) => {
  if (state.activeAccountId !== lastActiveAccountId) {
    lastActiveAccountId = state.activeAccountId;
    useAssistantStore.getState().reset();
  }
});
