import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createElement, type ReactNode } from "react";

const detail = {
  uid: 4, folder_name: "INBOX", subject: "Contract", from_address: "anu@altacee.com", from_name: "Anu",
  to_addresses: [], cc_addresses: [], date: "2026-09-17", html: null, text: "Please sign",
  raw_headers: "Message-ID: <abc@x>\r\n",
};
const fetchMessage = vi.fn();
vi.mock("@/hooks/useMessages", () => ({
  fetchMessage: (...args: unknown[]) => fetchMessage(...args),
  useBulkMoveMessages: () => ({ mutateAsync: vi.fn() }),
  useBulkUpdateFlags: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/useFilters", () => ({ useCreateFilter: () => ({ mutateAsync: vi.fn() }) }));
vi.mock("@/hooks/useIdentities", () => ({ useIdentities: () => ({ data: [] }) }));

import { ActionCard, DraftCard } from "@/components/assistant/ProposalCard";
import { useComposeStore } from "@/stores/useComposeStore";

function withQueryClient(children: ReactNode) {
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return createElement(QueryClientProvider, { client: queryClient }, children);
}

describe("DraftCard", () => {
  beforeEach(() => {
    fetchMessage.mockReset();
    useComposeStore.setState({ isOpen: false, to: "", inReplyTo: null, body: "" } as never);
  });

  it("does not fetch the message just by rendering", () => {
    render(withQueryClient(<DraftCard draft={{ ref: "m1", folder: "INBOX", uid: 4, body: "Signed, thanks." }} />));
    expect(fetchMessage).not.toHaveBeenCalled();
  });

  it("fetches on click and opens compose as a reply with the drafted body", async () => {
    fetchMessage.mockResolvedValue(detail);
    render(withQueryClient(<DraftCard draft={{ ref: "m1", folder: "INBOX", uid: 4, body: "Signed, thanks." }} />));
    fireEvent.click(screen.getByRole("button", { name: "Open in compose" }));

    await waitFor(() => expect(useComposeStore.getState().isOpen).toBe(true));
    expect(fetchMessage).toHaveBeenCalledWith(expect.anything(), "INBOX", 4);
    const s = useComposeStore.getState();
    expect(s.to).toBe("anu@altacee.com");
    expect(s.inReplyTo).toBe("<abc@x>");
    expect(s.body).toBe("Signed, thanks.");
  });

  it("shows an inline error when the fetch fails", async () => {
    fetchMessage.mockRejectedValue(new Error("Could not load message"));
    render(withQueryClient(<DraftCard draft={{ ref: "m1", folder: "INBOX", uid: 4, body: "Signed, thanks." }} />));
    fireEvent.click(screen.getByRole("button", { name: "Open in compose" }));
    expect((await screen.findByRole("alert")).textContent).toBe("Could not load message");
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
