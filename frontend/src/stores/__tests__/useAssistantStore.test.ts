import { beforeEach, describe, expect, it } from "vitest";
import { useAssistantStore } from "@/stores/useAssistantStore";
import { useAuthStore } from "@/stores/useAuthStore";
import { useUiStore } from "@/stores/useUiStore";

describe("useAssistantStore", () => {
  beforeEach(() => {
    localStorage.clear();
    useAssistantStore.setState({ open: false, turns: [], busy: false, abortController: null });
    useAuthStore.setState({ accounts: [], activeAccountId: "acct-a" });
    useUiStore.setState({ viewMode: "settings" });
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

  it("opening the panel via toggle switches to mail view; closing leaves the view alone", () => {
    useUiStore.setState({ viewMode: "settings" });
    useAssistantStore.getState().toggle(); // opens
    expect(useAssistantStore.getState().open).toBe(true);
    expect(useUiStore.getState().viewMode).toBe("mail");

    useUiStore.setState({ viewMode: "contacts" });
    useAssistantStore.getState().toggle(); // closes
    expect(useAssistantStore.getState().open).toBe(false);
    expect(useUiStore.getState().viewMode).toBe("contacts");
  });

  it("stamps a turn with the active account id", () => {
    useAuthStore.setState({ activeAccountId: "acct-a" });
    const id = useAssistantStore.getState().startTurn("hi", null);
    expect(useAssistantStore.getState().turns.find((t) => t.id === id)?.accountId).toBe("acct-a");
  });

  it("reset() aborts any in-flight request and clears turns and busy", () => {
    useAssistantStore.getState().startTurn("hi", null);
    const controller = useAssistantStore.getState().beginRequest();
    expect(controller).not.toBeNull();
    useAssistantStore.getState().reset();
    expect(controller!.signal.aborted).toBe(true);
    expect(useAssistantStore.getState().turns).toHaveLength(0);
    expect(useAssistantStore.getState().busy).toBe(false);
    expect(useAssistantStore.getState().abortController).toBeNull();
  });

  it("clear() also aborts an in-flight request", () => {
    useAssistantStore.getState().startTurn("hi", null);
    const controller = useAssistantStore.getState().beginRequest();
    useAssistantStore.getState().clear();
    expect(controller!.signal.aborted).toBe(true);
    expect(useAssistantStore.getState().turns).toHaveLength(0);
  });

  it("switching the active account resets turns and aborts the in-flight request", () => {
    useAuthStore.setState({ activeAccountId: "acct-a" });
    useAssistantStore.getState().startTurn("hi", null);
    const controller = useAssistantStore.getState().beginRequest();
    useAuthStore.setState({ activeAccountId: "acct-b" });
    expect(controller!.signal.aborted).toBe(true);
    expect(useAssistantStore.getState().turns).toHaveLength(0);
    expect(useAssistantStore.getState().busy).toBe(false);
  });
});
