import { describe, expect, it, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import { runAction, ActionCard, DraftCard } from "@/components/assistant/ProposalCard";
import { useAuthStore } from "@/stores/useAuthStore";

vi.mock("@/hooks/useMessages", () => ({
  fetchMessage: vi.fn(),
  useBulkMoveMessages: () => ({ mutateAsync: vi.fn() }),
  useBulkUpdateFlags: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/hooks/useFilters", () => ({ useCreateFilter: () => ({ mutateAsync: vi.fn() }) }));
vi.mock("@/hooks/useIdentities", () => ({ useIdentities: () => ({ data: [] }) }));
vi.mock("@tanstack/react-query", () => ({ useQueryClient: () => ({}) }));

const deps = () => ({ bulkMove: vi.fn().mockResolvedValue({}), bulkFlags: vi.fn().mockResolvedValue({}), createFilter: vi.fn().mockResolvedValue({}) });

describe("runAction", () => {
  it("moves grouped by source folder", async () => {
    const d = deps();
    await runAction({ id: "a", kind: "move", summary: "", folder: "Finance", messages: [
      { folder: "INBOX", uid: 1, subject: "" }, { folder: "INBOX", uid: 2, subject: "" }, { folder: "Junk", uid: 5, subject: "" },
    ] }, d);
    expect(d.bulkMove).toHaveBeenCalledWith({ fromFolder: "INBOX", toFolder: "Finance", uids: [1, 2] });
    expect(d.bulkMove).toHaveBeenCalledWith({ fromFolder: "Junk", toFolder: "Finance", uids: [5] });
  });

  it("marks read with the \\Seen flag", async () => {
    const d = deps();
    await runAction({ id: "a", kind: "mark_read", summary: "", messages: [{ folder: "INBOX", uid: 3, subject: "" }] }, d);
    expect(d.bulkFlags).toHaveBeenCalledWith({ folder: "INBOX", uids: [3], flags: ["\\Seen"], add: true });
  });

  it("creates an exact-address move filter", async () => {
    const d = deps();
    await runAction({ id: "a", kind: "create_filter", summary: "", messages: [], filter: { from: "billing@aws.example", folder: "Finance" } }, d);
    expect(d.createFilter).toHaveBeenCalledWith({
      name: "From billing@aws.example",
      conditions: [{ field: "from", op: "equals", value: "billing@aws.example" }],
      match_mode: "all",
      actions: [{ action_type: "move", action_value: "Finance" }],
    });
  });

});

describe("stale-account cards refuse to act", () => {
  beforeEach(() => {
    useAuthStore.setState({ accounts: [], activeAccountId: "acct-b" });
  });

  it("disables Confirm on an ActionCard stamped with a different account", () => {
    render(
      <ActionCard
        accountId="acct-a"
        action={{ id: "a", kind: "archive", summary: "Archive 1 email", messages: [] }}
      />,
    );
    expect((screen.getByRole("button", { name: "Confirm" }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByText(/different account/i)).toBeTruthy();
  });

  it("leaves Confirm enabled when the account still matches", () => {
    render(
      <ActionCard
        accountId="acct-b"
        action={{ id: "a", kind: "archive", summary: "Archive 1 email", messages: [] }}
      />,
    );
    expect((screen.getByRole("button", { name: "Confirm" }) as HTMLButtonElement).disabled).toBe(false);
  });

  it("disables Open in compose on a DraftCard stamped with a different account", () => {
    render(<DraftCard accountId="acct-a" draft={{ ref: "m1", folder: "INBOX", uid: 1, body: "hi" }} />);
    expect((screen.getByRole("button", { name: /open in compose/i }) as HTMLButtonElement).disabled).toBe(true);
  });
});
