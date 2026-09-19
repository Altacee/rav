import { describe, expect, it, vi } from "vitest";
import { runAction } from "@/components/assistant/ProposalCard";

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
