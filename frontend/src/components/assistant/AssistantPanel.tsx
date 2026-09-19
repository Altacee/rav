"use client";
import { useEffect, useRef, useState } from "react";
import { ArrowUp, Sparkles, Square, X } from "lucide-react";
import { useAssistantChat, useAssistantStatus } from "@/hooks/useAssistant";
import { useMessage } from "@/hooks/useMessages";
import { useIsMobile } from "@/hooks/useIsMobile";
import { useAssistantStore } from "@/stores/useAssistantStore";
import { cn } from "@/lib/utils";
import { AnswerText } from "./AnswerText";
import { ActionCard, DraftCard } from "./ProposalCard";

const SUGGESTIONS_WITH_EMAIL = ["Summarise this", "Draft a reply", "What needs my reply this week?"];
const SUGGESTIONS = ["What needs my reply this week?", "Any invoices due soon?", "What did I miss today?"];

/** ⌘J / Ctrl+J toggles the panel when the assistant is enabled. */
export function useAssistantShortcut(enabled: boolean) {
  useEffect(() => {
    if (!enabled) return;
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "j") {
        e.preventDefault();
        useAssistantStore.getState().toggle();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [enabled]);
}

export function AssistantPanel() {
  const { enabled } = useAssistantStatus();
  const { ask, stop, busy, context } = useAssistantChat();
  const turns = useAssistantStore((s) => s.turns);
  const setOpen = useAssistantStore((s) => s.setOpen);
  const isMobile = useIsMobile();
  const [draft, setDraft] = useState("");
  const [useContext, setUseContext] = useState(true);
  const ctxKey = context ? `${context.folder}/${context.uid}` : "";
  const [prevCtxKey, setPrevCtxKey] = useState(ctxKey);
  if (ctxKey !== prevCtxKey) {
    // The open email changed: re-arm "About: …" for the new one (React's
    // recommended adjust-state-during-render pattern, no effect needed).
    setPrevCtxKey(ctxKey);
    setUseContext(true);
  }
  const { data: openMsg } = useMessage(context?.folder ?? "", context?.uid ?? 0);
  const endRef = useRef<HTMLDivElement>(null);

  useEffect(() => endRef.current?.scrollIntoView?.({ block: "end" }), [turns]);

  if (!enabled) return null;
  const withEmail = !!context && useContext;

  const send = (q: string) => {
    const question = q.trim();
    if (!question || busy) return;
    setDraft("");
    void ask(question, withEmail);
  };

  return (
    <aside
      aria-label="Assistant"
      className={cn(
        "flex min-h-0 flex-col border-l border-border bg-background",
        isMobile ? "fixed inset-0 z-50" : "h-full w-[380px] shrink-0",
      )}
      onKeyDown={(e) => e.key === "Escape" && setOpen(false)}
    >
      <header className="flex h-12 items-center justify-between border-b border-border px-3">
        <span className="flex items-center gap-2 text-sm font-medium">
          <Sparkles className="size-4 text-primary" aria-hidden /> Assistant
        </span>
        <button type="button" aria-label="Close assistant" className="p-1 text-muted-foreground hover:text-foreground" onClick={() => setOpen(false)}>
          <X className="size-4" />
        </button>
      </header>

      <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-3" aria-live="polite">
        {turns.length === 0 && (
          <div className="space-y-2">
            {(withEmail ? SUGGESTIONS_WITH_EMAIL : SUGGESTIONS).map((s) => (
              <button key={s} type="button" onClick={() => send(s)}
                className="block w-full border border-border px-3 py-2 text-left text-sm hover:border-primary/60">
                {s}
              </button>
            ))}
          </div>
        )}
        {turns.map((t) => (
          <div key={t.id} className="space-y-2">
            <p className="ml-8 bg-muted px-3 py-2 text-sm">{t.question}</p>
            {t.status && <p className="text-xs text-muted-foreground">{t.status}</p>}
            {t.answer && <AnswerText text={t.answer} sources={t.sources} />}
            {t.drafts.map((d) => <DraftCard key={`${d.ref}-${d.body.length}`} draft={d} accountId={t.accountId} />)}
            {t.actions.map((a) => <ActionCard key={a.id} action={a} accountId={t.accountId} />)}
            {t.error && (
              <div role="alert" className="flex items-center justify-between border border-destructive/50 px-3 py-2 text-xs text-destructive">
                {t.error.message}
                {t.error.retryable && (
                  <button type="button" className="underline" onClick={() => send(t.question)}>Retry</button>
                )}
              </div>
            )}
          </div>
        ))}
        <div ref={endRef} />
      </div>

      <div className="border-t border-border p-3">
        {withEmail && openMsg && (
          <div className="mb-2 flex items-center justify-between border border-border px-2 py-1 text-xs text-muted-foreground">
            <span className="truncate">About: {openMsg.subject} · {openMsg.from_name || openMsg.from_address}</span>
            <button type="button" aria-label="Ask about the whole mailbox instead" onClick={() => setUseContext(false)}>
              <X className="size-3" />
            </button>
          </div>
        )}
        <div className="flex items-end gap-2">
          <textarea
            aria-label="Ask the assistant"
            rows={2}
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                send(draft);
              }
            }}
            placeholder="Ask about your mail…"
            className="min-h-10 flex-1 resize-none border border-border bg-transparent px-2 py-1.5 text-sm outline-none focus:border-primary"
          />
          {busy ? (
            <button type="button" aria-label="Stop" onClick={stop} className="border border-border p-2"><Square className="size-4" /></button>
          ) : (
            <button type="button" aria-label="Send" onClick={() => send(draft)} disabled={!draft.trim()} className="bg-primary p-2 text-primary-foreground disabled:opacity-40"><ArrowUp className="size-4" /></button>
          )}
        </div>
      </div>
    </aside>
  );
}
