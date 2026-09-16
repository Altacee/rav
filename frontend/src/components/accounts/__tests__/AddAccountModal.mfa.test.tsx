import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { AddAccountModal } from "../AddAccountModal";

const { mockApiPost, mockFetchAccounts } = vi.hoisted(() => ({
  mockApiPost: vi.fn(),
  mockFetchAccounts: vi.fn(),
}));

vi.mock("@/lib/api", () => ({
  apiPost: mockApiPost,
  fetchAccounts: mockFetchAccounts,
}));

function renderModal() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <AddAccountModal open onClose={() => {}} />
    </QueryClientProvider>,
  );
}

async function submitCredentials() {
  fireEvent.change(screen.getByLabelText(/email/i), {
    target: { value: "aditya@altacee.dev" },
  });
  fireEvent.change(screen.getByLabelText(/^password/i), {
    target: { value: "hunter2" },
  });
  fireEvent.click(screen.getByRole("button", { name: /sign in/i }));
}

describe("AddAccountModal with two-factor accounts", () => {
  beforeEach(() => {
    mockApiPost.mockReset();
    mockFetchAccounts.mockReset();
    mockFetchAccounts.mockResolvedValue({ accounts: [] });
  });

  it("asks for the code when the server says MFA is required", async () => {
    mockApiPost.mockResolvedValueOnce({ mfa_required: true, mfa_type: "totp" });
    renderModal();
    await submitCredentials();

    await waitFor(() => {
      expect(screen.getByLabelText(/verification code/i)).toBeTruthy();
    });
    // The old code read response.account.id unconditionally and blew up.
    expect(screen.queryByText(/reading 'id'/i)).toBeNull();
    expect(screen.queryByText(/unexpected error/i)).toBeNull();
  });

  it("re-submits with the code and finishes sign-in", async () => {
    mockApiPost
      .mockResolvedValueOnce({ mfa_required: true, mfa_type: "totp" })
      .mockResolvedValueOnce({
        account: { id: "acc-1", email: "aditya@altacee.dev", imapHost: "h", smtpHost: "h" },
      });
    mockFetchAccounts.mockResolvedValue({
      accounts: [{ id: "acc-1", email: "aditya@altacee.dev", imapHost: "h", smtpHost: "h" }],
    });
    renderModal();
    await submitCredentials();

    const code = await screen.findByLabelText(/verification code/i);
    fireEvent.change(code, { target: { value: "123 456" } });
    fireEvent.click(screen.getByRole("button", { name: /verify|sign in/i }));

    await waitFor(() => {
      expect(mockApiPost).toHaveBeenCalledTimes(2);
    });
    const secondCall = mockApiPost.mock.calls[1][1] as Record<string, unknown>;
    expect(secondCall.totp_code).toBe("123456");
  });

  it("reports a response without an account instead of throwing a TypeError", async () => {
    mockApiPost.mockResolvedValueOnce({ something: "unexpected" });
    renderModal();
    await submitCredentials();

    await waitFor(() => {
      expect(screen.getByText(/could not be completed/i)).toBeTruthy();
    });
  });
});
