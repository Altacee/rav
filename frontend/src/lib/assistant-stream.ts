import type { AssistantEvent } from "@/types/assistant";

/** Incremental text/event-stream parser: feed chunks, get complete events. */
export class SseParser {
  private buffer = "";

  push(chunk: string): { event: string; data: string }[] {
    this.buffer += chunk.replace(/\r\n/g, "\n");
    const out: { event: string; data: string }[] = [];
    let end: number;
    while ((end = this.buffer.indexOf("\n\n")) !== -1) {
      const block = this.buffer.slice(0, end);
      this.buffer = this.buffer.slice(end + 2);
      let event = "message";
      const data: string[] = [];
      for (const line of block.split("\n")) {
        if (line.startsWith("event:")) event = line.slice(6).trim();
        else if (line.startsWith("data:")) data.push(line.slice(5).trimStart());
      }
      if (data.length) out.push({ event, data: data.join("\n") });
    }
    return out;
  }
}

export function parseAssistantEvent(event: string, data: string): AssistantEvent | null {
  let v: unknown;
  try {
    v = JSON.parse(data);
  } catch {
    return null;
  }
  const o = v as Record<string, unknown>;
  switch (event) {
    case "status":
      return { type: "status", text: String(o.text ?? "") };
    case "text":
      return { type: "text", delta: String(o.delta ?? "") };
    case "sources":
      return Array.isArray(v) ? { type: "sources", sources: v as never } : null;
    case "draft":
      return { type: "draft", draft: o as never };
    case "action":
      return { type: "action", action: o as never };
    case "error":
      return {
        type: "error",
        message: String(o.message ?? "Something went wrong"),
        retryable: Boolean(o.retryable),
      };
    case "done":
      return {
        type: "done",
        inputTokens: Number(o.input_tokens ?? 0),
        outputTokens: Number(o.output_tokens ?? 0),
      };
    default:
      return null;
  }
}
