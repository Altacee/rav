import { describe, expect, it } from "vitest";
import { SseParser, parseAssistantEvent } from "@/lib/assistant-stream";

describe("SseParser", () => {
  it("splits events across chunk boundaries", () => {
    const p = new SseParser();
    expect(p.push('event: text\ndata: {"del')).toEqual([]);
    expect(p.push('ta":"Hi"}\n\nevent: done\ndata: {"input_tokens":1,"output_tokens":2}\n\n')).toEqual([
      { event: "text", data: '{"delta":"Hi"}' },
      { event: "done", data: '{"input_tokens":1,"output_tokens":2}' },
    ]);
  });

  it("ignores keep-alive comments and handles CRLF", () => {
    const p = new SseParser();
    expect(p.push(":\r\n\r\nevent: status\r\ndata: {\"text\":\"Searching\"}\r\n\r\n")).toEqual([
      { event: "status", data: '{"text":"Searching"}' },
    ]);
  });
});

describe("parseAssistantEvent", () => {
  it("maps each event to a typed value", () => {
    expect(parseAssistantEvent("text", '{"delta":"a"}')).toEqual({ type: "text", delta: "a" });
    expect(parseAssistantEvent("done", '{"input_tokens":3,"output_tokens":4}')).toEqual({ type: "done", inputTokens: 3, outputTokens: 4 });
    expect(parseAssistantEvent("sources", '[{"ref":"m1","folder":"INBOX","uid":2,"subject":"s","from":"f","date":"d"}]'))
      .toEqual({ type: "sources", sources: [{ ref: "m1", folder: "INBOX", uid: 2, subject: "s", from: "f", date: "d" }] });
    expect(parseAssistantEvent("action", '{"id":"x","kind":"move","summary":"Move 1 email to A","messages":[],"folder":"A"}'))
      .toMatchObject({ type: "action", action: { kind: "move", folder: "A" } });
  });

  it("drops unknown events and bad JSON", () => {
    expect(parseAssistantEvent("mystery", "{}")).toBeNull();
    expect(parseAssistantEvent("text", "{nope")).toBeNull();
  });
});
