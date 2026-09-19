import { beforeEach, describe, expect, it } from "vitest";
import { useAssistantStore } from "@/stores/useAssistantStore";

describe("useAssistantStore", () => {
  beforeEach(() => {
    localStorage.clear();
    useAssistantStore.setState({ open: false, turns: [] });
  });

  it("folds streamed events into a turn", () => {
    const s = useAssistantStore.getState();
    const id = s.startTurn("what did anu say?", null);
    s.applyEvent(id, { type: "status", text: "Searching mail…" });
    s.applyEvent(id, { type: "text", delta: "She needs " });
    s.applyEvent(id, { type: "text", delta: "a signature [m1]." });
    s.applyEvent(id, { type: "sources", sources: [{ ref: "m1", folder: "INBOX", uid: 4, subject: "Contract", from: "Anu", date: "d" }] });
    s.applyEvent(id, { type: "action", action: { id: "a", kind: "archive", summary: "Archive 1 email", messages: [], folder: "Archive" } });
    s.applyEvent(id, { type: "done", inputTokens: 1, outputTokens: 1 });
    const t = useAssistantStore.getState().turns[0];
    expect(t.answer).toBe("She needs a signature [m1].");
    expect(t.status).toBeNull();
    expect(t.sources[0].uid).toBe(4);
    expect(t.actions).toHaveLength(1);
    expect(t.done).toBe(true);
  });

  it("records errors and ends the turn", () => {
    const s = useAssistantStore.getState();
    const id = s.startTurn("hi", null);
    s.applyEvent(id, { type: "error", message: "The assistant can't reach OpenAI right now.", retryable: true });
    const t = useAssistantStore.getState().turns[0];
    expect(t.error).toEqual({ message: "The assistant can't reach OpenAI right now.", retryable: true });
    expect(t.done).toBe(true);
  });

  it("remembers whether the panel is open", () => {
    useAssistantStore.getState().toggle();
    expect(useAssistantStore.getState().open).toBe(true);
    expect(localStorage.getItem("assistant-open")).toBe("1");
  });
});
