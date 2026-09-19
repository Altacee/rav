import { useCallback, useMemo } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPostStream } from "@/lib/api";
import { SseParser, parseAssistantEvent } from "@/lib/assistant-stream";
import { resolveFolderId } from "@/lib/folders";
import { useAssistantStore, type AssistantTurn } from "@/stores/useAssistantStore";
import { useUiStore } from "@/stores/useUiStore";

export function useAssistantStatus(): { enabled: boolean } {
  const { data } = useQuery({
    queryKey: ["assistant-status"],
    queryFn: () => apiGet<{ enabled: boolean }>("/assistant/status"),
    staleTime: 5 * 60_000,
  });
  return { enabled: data?.enabled ?? false };
}

/** Mirrors the backend's `MAX_HISTORY`/`MAX_CONTENT` in backend/src/routes/assistant.rs. */
const MAX_HISTORY_MESSAGES = 19;
const MAX_CONTENT_CHARS = 8000;

interface ChatMessagePayload {
  role: "user" | "assistant";
  content: string;
}

/**
 * Builds the message list sent to /assistant/chat: at most the last
 * MAX_HISTORY_MESSAGES prior messages plus the new question, each capped at
 * MAX_CONTENT_CHARS characters. One long answer must not make every later
 * question fail the backend's 400 on oversized history.
 */
export function buildChatMessages(turns: AssistantTurn[], question: string): ChatMessagePayload[] {
  const history = turns
    .filter((t) => t.done && !t.error)
    .flatMap((t): ChatMessagePayload[] => [
      { role: "user", content: t.question },
      { role: "assistant", content: t.answer },
    ]);
  const messages = [...history.slice(-MAX_HISTORY_MESSAGES), { role: "user" as const, content: question }];
  return messages.map((m) => ({ ...m, content: m.content.slice(0, MAX_CONTENT_CHARS) }));
}

export function useAssistantChat(): {
  ask: (question: string, useContext: boolean) => Promise<void>;
  stop: () => void;
  busy: boolean;
  context: { folder: string; uid: number } | null;
} {
  const queryClient = useQueryClient();
  const busy = useAssistantStore((s) => s.busy);
  const activeFolder = useUiStore((s) => s.activeFolder);
  const selectedUid = useUiStore((s) => s.selectedMessageUid);
  const context = useMemo(
    () => (selectedUid !== null ? { folder: activeFolder, uid: selectedUid } : null),
    [activeFolder, selectedUid],
  );

  const stop = useCallback(() => useAssistantStore.getState().stopRequest(), []);

  const ask = useCallback(
    async (question: string, useContext: boolean) => {
      const store = useAssistantStore.getState();
      if (store.busy) return; // a turn is already in flight; the panel's Send is disabled too, but refuse defensively
      const ctx = useContext ? context : null;
      const messages = buildChatMessages(store.turns, question);
      const id = store.startTurn(question, ctx);

      let open: { folder_id: string; uid: number } | null = null;
      if (ctx) {
        try {
          open = { folder_id: resolveFolderId(queryClient, ctx.folder), uid: ctx.uid };
        } catch {
          open = null; // folder list not loaded: ask about the whole mailbox
        }
      }

      const controller = useAssistantStore.getState().beginRequest();
      if (!controller) return; // lost the race to another ask() call
      try {
        const res = await apiPostStream("/assistant/chat", { messages, open }, controller.signal);
        const reader = res.body!.pipeThrough(new TextDecoderStream()).getReader();
        const parser = new SseParser();
        for (;;) {
          const { value, done } = await reader.read();
          if (done) break;
          for (const { event, data } of parser.push(value)) {
            const e = parseAssistantEvent(event, data);
            if (e) useAssistantStore.getState().applyEvent(id, e);
          }
        }
        const turn = useAssistantStore.getState().turns.find((t) => t.id === id);
        if (turn && !turn.done) {
          useAssistantStore.getState().applyEvent(id, { type: "done", inputTokens: 0, outputTokens: 0 });
        }
      } catch (err) {
        if (controller.signal.aborted) {
          useAssistantStore.getState().applyEvent(id, { type: "done", inputTokens: 0, outputTokens: 0 });
        } else {
          useAssistantStore.getState().failTurn(id, err instanceof Error ? err.message : "Something went wrong", true);
        }
      } finally {
        useAssistantStore.getState().endRequest();
      }
    },
    [context, queryClient],
  );

  return { ask, stop, busy, context };
}
