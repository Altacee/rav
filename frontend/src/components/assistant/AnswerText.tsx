"use client";
import type { ReactNode } from "react";
import type { AssistantSource } from "@/types/assistant";
import { useUiStore } from "@/stores/useUiStore";

const REF = /\[(m\d+)\]/g;
// Bold before single-star em so "**x**" isn't read as two unmatched "*x*"s.
const INLINE = /\*\*(.+?)\*\*|\*(.+?)\*|_(.+?)_|`(.+?)`|\[(m\d+)\]/g;

type Block = { type: "p"; lines: string[] } | { type: "ul" | "ol"; items: string[] };

const BULLET = /^\s*[-*]\s+(.*)$/;
const NUMBERED = /^\s*\d+\.\s+(.*)$/;

/** Splits raw model text into paragraph/list blocks. Pure, no React. */
function toBlocks(text: string): Block[] {
  const blocks: Block[] = [];
  for (const para of text.split(/\n{2,}/)) {
    const lines = para.split("\n");
    let i = 0;
    while (i < lines.length) {
      const bulletMatch = lines[i].match(BULLET);
      const numberedMatch = lines[i].match(NUMBERED);
      if (bulletMatch) {
        const items: string[] = [];
        while (i < lines.length) {
          const m = lines[i].match(BULLET);
          if (!m) break;
          items.push(m[1]);
          i++;
        }
        blocks.push({ type: "ul", items });
      } else if (numberedMatch) {
        const items: string[] = [];
        while (i < lines.length) {
          const m = lines[i].match(NUMBERED);
          if (!m) break;
          items.push(m[1]);
          i++;
        }
        blocks.push({ type: "ol", items });
      } else {
        const textLines: string[] = [];
        while (i < lines.length && !BULLET.test(lines[i]) && !NUMBERED.test(lines[i])) {
          textLines.push(lines[i]);
          i++;
        }
        if (textLines.some((l) => l.length > 0)) blocks.push({ type: "p", lines: textLines });
      }
    }
  }
  return blocks;
}

function renderChip(ref: string, key: string, byRef: Map<string, AssistantSource>, numbers: Map<string, number>): ReactNode {
  const src = byRef.get(ref);
  if (!src) return `[${ref}]`;
  return (
    <button
      key={key}
      type="button"
      title={`${src.subject} · ${src.from}`}
      aria-label={`Source ${numbers.get(ref)}: ${src.subject}`}
      className="mx-0.5 inline-flex h-4 min-w-4 items-center justify-center border border-primary/60 px-1 align-text-top text-[10px] font-medium text-primary hover:bg-primary/10"
      onClick={() => {
        const ui = useUiStore.getState();
        ui.setActiveFolder(src.folder);
        ui.selectMessage(src.uid);
      }}
    >
      {numbers.get(ref)}
    </button>
  );
}

/** Renders inline markdown (bold/em/code/[mN] chips) as React nodes only —
 * never HTML strings. Unmatched/unclosed markers (streaming) just fall back
 * to literal text instead of throwing. */
function renderInline(text: string, byRef: Map<string, AssistantSource>, numbers: Map<string, number>, keyBase: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let last = 0;
  let i = 0;
  for (const m of text.matchAll(INLINE)) {
    if (m.index! > last) nodes.push(text.slice(last, m.index));
    const key = `${keyBase}-${i++}`;
    if (m[1] !== undefined) nodes.push(<strong key={key}>{renderInline(m[1], byRef, numbers, key)}</strong>);
    else if (m[2] !== undefined) nodes.push(<em key={key}>{renderInline(m[2], byRef, numbers, key)}</em>);
    else if (m[3] !== undefined) nodes.push(<em key={key}>{renderInline(m[3], byRef, numbers, key)}</em>);
    else if (m[4] !== undefined) nodes.push(<code key={key} className="bg-muted px-1 text-[13px]">{m[4]}</code>);
    else if (m[5] !== undefined) nodes.push(renderChip(m[5], key, byRef, numbers));
    last = m.index! + m[0].length;
  }
  if (last < text.length) nodes.push(text.slice(last));
  return nodes;
}

function renderBlock(block: Block, bi: number, byRef: Map<string, AssistantSource>, numbers: Map<string, number>): ReactNode {
  if (block.type === "p") {
    const children: ReactNode[] = [];
    block.lines.forEach((line, li) => {
      if (li > 0) children.push(<br key={`br-${bi}-${li}`} />);
      children.push(...renderInline(line, byRef, numbers, `${bi}-${li}`));
    });
    return (
      <p key={bi} className="text-sm leading-relaxed">
        {children}
      </p>
    );
  }
  const Tag = block.type === "ul" ? "ul" : "ol";
  return (
    <Tag key={bi} className={block.type === "ul" ? "list-disc space-y-1 pl-4 text-sm leading-relaxed" : "list-decimal space-y-1 pl-4 text-sm leading-relaxed"}>
      {block.items.map((item, ii) => <li key={ii}>{renderInline(item, byRef, numbers, `${bi}-${ii}`)}</li>)}
    </Tag>
  );
}

/** Answer text rendered as minimal, XSS-safe markdown: paragraphs, bullet
 * and numbered lists, bold/em/code, and [mN] source chips numbered by first
 * appearance across the whole answer. */
export function AnswerText({ text, sources }: { text: string; sources: AssistantSource[] }) {
  const byRef = new Map(sources.map((s) => [s.ref, s]));
  const numbers = new Map<string, number>();
  for (const m of text.matchAll(REF)) {
    if (byRef.has(m[1]) && !numbers.has(m[1])) numbers.set(m[1], numbers.size + 1);
  }
  const blocks = toBlocks(text);
  return <div className="space-y-2">{blocks.map((b, bi) => renderBlock(b, bi, byRef, numbers))}</div>;
}
