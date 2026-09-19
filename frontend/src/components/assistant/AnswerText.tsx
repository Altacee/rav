"use client";
import { Fragment } from "react";
import type { AssistantSource } from "@/types/assistant";
import { useUiStore } from "@/stores/useUiStore";

const REF = /\[(m\d+)\]/g;

/** Answer text with [mN] refs turned into numbered chips that open the email. */
export function AnswerText({ text, sources }: { text: string; sources: AssistantSource[] }) {
  const byRef = new Map(sources.map((s) => [s.ref, s]));
  const numbers = new Map<string, number>();
  const parts: React.ReactNode[] = [];
  let last = 0;
  for (const m of text.matchAll(REF)) {
    const src = byRef.get(m[1]);
    if (!src) continue;
    if (!numbers.has(src.ref)) numbers.set(src.ref, numbers.size + 1);
    parts.push(text.slice(last, m.index));
    parts.push(
      <button
        key={`${m.index}`}
        type="button"
        title={`${src.subject} · ${src.from}`}
        aria-label={`Source ${numbers.get(src.ref)}: ${src.subject}`}
        className="mx-0.5 inline-flex h-4 min-w-4 items-center justify-center border border-primary/60 px-1 align-text-top text-[10px] font-medium text-primary hover:bg-primary/10"
        onClick={() => {
          const ui = useUiStore.getState();
          ui.setActiveFolder(src.folder);
          ui.selectMessage(src.uid);
        }}
      >
        {numbers.get(src.ref)}
      </button>,
    );
    last = (m.index ?? 0) + m[0].length;
  }
  parts.push(text.slice(last));
  return <p className="whitespace-pre-wrap text-sm leading-relaxed">{parts.map((p, i) => <Fragment key={i}>{p}</Fragment>)}</p>;
}
