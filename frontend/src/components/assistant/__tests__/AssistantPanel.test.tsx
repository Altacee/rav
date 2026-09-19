import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

const ask = vi.fn();
vi.mock("@/hooks/useAssistant", () => ({
  useAssistantStatus: () => ({ enabled: true }),
  useAssistantChat: () => ({ ask, stop: vi.fn(), busy: false, context: { folder: "INBOX", uid: 4 } }),
}));
vi.mock("@/hooks/useMessages", () => ({
  useMessage: () => ({ data: { subject: "Contract", from_name: "Anu", from_address: "anu@altacee.com" } }),
  useBulkMoveMessages: () => ({ mutateAsync: vi.fn() }),
  useBulkUpdateFlags: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/useFilters", () => ({ useCreateFilter: () => ({ mutateAsync: vi.fn() }) }));
vi.mock("@/hooks/useIdentities", () => ({ useIdentities: () => ({ data: [] }) }));

import { AssistantPanel } from "@/components/assistant/AssistantPanel";
import { useAssistantStore } from "@/stores/useAssistantStore";

describe("AssistantPanel", () => {
  beforeEach(() => {
    ask.mockReset();
    useAssistantStore.setState({ open: true, turns: [] });
  });

  it("shows the open email as context and suggestions", () => {
    render(<AssistantPanel />);
    expect(screen.getByText(/About: Contract · Anu/)).toBeTruthy();
    expect(screen.getByRole("button", { name: "Summarise this" })).toBeTruthy();
  });

  it("sends with Enter and keeps Shift+Enter as a newline", () => {
    render(<AssistantPanel />);
    const box = screen.getByRole("textbox", { name: "Ask the assistant" });
    fireEvent.change(box, { target: { value: "what's due?" } });
    fireEvent.keyDown(box, { key: "Enter", shiftKey: true });
    expect(ask).not.toHaveBeenCalled();
    fireEvent.keyDown(box, { key: "Enter" });
    expect(ask).toHaveBeenCalledWith("what's due?", true);
  });

  it("removing the context chip asks about the whole mailbox", () => {
    render(<AssistantPanel />);
    fireEvent.click(screen.getByRole("button", { name: "Ask about the whole mailbox instead" }));
    fireEvent.click(screen.getByRole("button", { name: "What needs my reply this week?" }));
    expect(ask).toHaveBeenCalledWith("What needs my reply this week?", false);
  });

  it("renders streamed turns with cards", () => {
    useAssistantStore.setState({ open: true, turns: [{
      id: "t", question: "q", answer: "Archive it [m1].", status: null, done: true, error: null, context: null,
      sources: [{ ref: "m1", folder: "INBOX", uid: 1, subject: "S", from: "f", date: "d" }],
      drafts: [], actions: [{ id: "a", kind: "archive", summary: "Archive 1 email", messages: [], folder: "Archive" }],
    }] });
    render(<AssistantPanel />);
    expect(screen.getByText("Archive 1 email")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Confirm" })).toBeTruthy();
  });
});
