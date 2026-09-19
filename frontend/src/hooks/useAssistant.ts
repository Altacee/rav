import { useCallback, useMemo, useRef, useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { apiGet, apiPostStream } from "@/lib/api";
import { SseParser, parseAssistantEvent } from "@/lib/assistant-stream";
import { resolveFolderId } from "@/lib/folders";
import { useAssistantStore } from "@/stores/useAssistantStore";
import { useUiStore } from "@/stores/useUiStore";

export function useAssistantStatus(): { enabled: boolean } {
  const { data } = useQuery({
    queryKey: ["assistant-status"],
    queryFn: () => apiGet<{ enabled: boolean }>("/assistant/status"),
    staleTime: 5 * 60_000,
  });
  return { enabled: data?.enabled ?? false };
}

export function useAssistantChat(): {
  ask: (question: string, useContext: boolean) => Promise<void>;
  stop: () => void;
  busy: boolean;
  context: { folder: string; uid: number } | null;
} {
  const queryClient = useQueryClient();
  const abortRef = useRef<AbortController | null>(null);
  const [busy, setBusy] = useState(false);
  const activeFolder = useUiStore((s) => s.activeFolder);
  const selectedUid = useUiStore((s) => s.selectedMessageUid);
  const context = useMemo(
    () => (selectedUid !== null ? { folder: activeFolder, uid: selectedUid } : null),
    [activeFolder, selectedUid],
  );

  const stop = useCallback(() => abortRef.current?.abort(), []);

  const ask = useCallback(
    async (question: string, useContext: boolean) => {
      const store = useAssistantStore.getState();
      const ctx = useContext ? context : null;
      const messages = store.turns
        .filter((t) => t.done && !t.error)
        .flatMap((t) => [
          { role: "user", content: t.question },
          { role: "assistant", content: t.answer },
        ]);
      messages.push({ role: "user", content: question });
      const id = store.startTurn(question, ctx);

      let open: { folder_id: string; uid: number } | null = null;
      if (ctx) {
        try {
          open = { folder_id: resolveFolderId(queryClient, ctx.folder), uid: ctx.uid };
        } catch {
          open = null; // folder list not loaded: ask about the whole mailbox
        }
      }

      const controller = new AbortController();
      abortRef.current = controller;
      setBusy(true);
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
        setBusy(false);
        abortRef.current = null;
      }
    },
    [context, queryClient],
  );

  return { ask, stop, busy, context };
}
