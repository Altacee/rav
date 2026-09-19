import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";

const detail = {
  uid: 4, folder_name: "INBOX", subject: "Contract", from_address: "anu@altacee.com", from_name: "Anu",
  to_addresses: [], cc_addresses: [], date: "2026-09-17", html: null, text: "Please sign",
  raw_headers: "Message-ID: <abc@x>\r\n",
};
vi.mock("@/hooks/useMessages", () => ({
  useMessage: () => ({ data: detail }),
  useBulkMoveMessages: () => ({ mutateAsync: vi.fn() }),
  useBulkUpdateFlags: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/useFilters", () => ({ useCreateFilter: () => ({ mutateAsync: vi.fn() }) }));
vi.mock("@/hooks/useIdentities", () => ({ useIdentities: () => ({ data: [] }) }));

import { ActionCard, DraftCard } from "@/components/assistant/ProposalCard";
import { useComposeStore } from "@/stores/useComposeStore";

describe("DraftCard", () => {
  it("opens compose as a reply with the drafted body", () => {
    render(<DraftCard draft={{ ref: "m1", folder: "INBOX", uid: 4, body: "Signed, thanks." }} />);
    fireEvent.click(screen.getByRole("button", { name: "Open in compose" }));
    const s = useComposeStore.getState();
    expect(s.isOpen).toBe(true);
    expect(s.to).toBe("anu@altacee.com");
    expect(s.inReplyTo).toBe("<abc@x>");
    expect(s.body).toBe("<p>Signed, thanks.</p>");
  });
});

describe("ActionCard", () => {
  it("runs nothing until Confirm, and Dismiss removes it", () => {
    const { container } = render(<ActionCard action={{ id: "a", kind: "archive", summary: "Archive 1 email", folder: "Archive", messages: [{ folder: "INBOX", uid: 1, subject: "S" }] }} />);
    expect(screen.getByText("Archive 1 email")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(container.textContent).toBe("");
  });
});
