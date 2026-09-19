import { describe, expect, it } from "vitest";
import { buildChatMessages } from "@/hooks/useAssistant";
import type { AssistantTurn } from "@/stores/useAssistantStore";

function turn(i: number, overrides: Partial<AssistantTurn> = {}): AssistantTurn {
  return {
    id: `t${i}`,
    question: `q${i}`,
    answer: `a${i}`,
    status: null,
    sources: [],
    drafts: [],
    actions: [],
    error: null,
    done: true,
    context: null,
    ...overrides,
  };
}

describe("buildChatMessages", () => {
  it("keeps at most the last 19 prior messages plus the new question", () => {
    // 15 done turns => 30 history messages; only the newest 19 plus the question (20 total) should survive.
    const turns = Array.from({ length: 15 }, (_, i) => turn(i));
    const messages = buildChatMessages(turns, "new question");
    expect(messages).toHaveLength(20);
    expect(messages[messages.length - 1]).toEqual({ role: "user", content: "new question" });
    // The oldest surviving message is the assistant answer from turn index 5 (30 - 19 = 11 dropped).
    expect(messages[0]).toEqual({ role: "assistant", content: "a5" });
  });

  it("truncates any message content to 8000 characters", () => {
    const long = "x".repeat(9000);
    const turns = [turn(0, { answer: long })];
    const messages = buildChatMessages(turns, "y".repeat(9000));
    expect(messages[1].content).toHaveLength(8000);
    expect(messages[messages.length - 1].content).toHaveLength(8000);
  });

  it("excludes turns that are not done or that errored", () => {
    const turns = [turn(0, { done: false }), turn(1, { error: { message: "x", retryable: true } })];
    const messages = buildChatMessages(turns, "q");
    expect(messages).toEqual([{ role: "user", content: "q" }]);
  });
});
