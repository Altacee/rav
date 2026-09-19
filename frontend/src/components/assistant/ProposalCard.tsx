"use client";
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { fetchMessage, useBulkMoveMessages, useBulkUpdateFlags } from "@/hooks/useMessages";
import { useCreateFilter } from "@/hooks/useFilters";
import { useIdentities } from "@/hooks/useIdentities";
import { buildReplyParams } from "@/lib/reply";
import { useComposeStore } from "@/stores/useComposeStore";
import { useAuthStore } from "@/stores/useAuthStore";
import type { ActionProposal, DraftProposal } from "@/types/assistant";
import type { CreateFilterRule } from "@/types/filter";

/** True once the account the turn was asked under is no longer the active
 * one — e.g. after switching mailboxes. A card in that state must refuse to
 * act: its refs and folder names resolve against the mailbox it was asked
 * about, not whichever one is open now. */
function useIsStaleAccount(accountId: string | null): boolean {
  const activeAccountId = useAuthStore((s) => s.activeAccountId);
  return accountId !== null && accountId !== activeAccountId;
}

type Deps = {
  bulkMove: (a: { fromFolder: string; toFolder: string; uids: number[] }) => Promise<unknown>;
  bulkFlags: (a: { folder: string; uids: number[]; flags: string[]; add: boolean }) => Promise<unknown>;
  createFilter: (r: CreateFilterRule) => Promise<unknown>;
};

function byFolder(action: ActionProposal): Map<string, number[]> {
  const m = new Map<string, number[]>();
  for (const msg of action.messages) m.set(msg.folder, [...(m.get(msg.folder) ?? []), msg.uid]);
  return m;
}

/** Runs a confirmed action through the existing mutations. Only called from Confirm. */
export async function runAction(action: ActionProposal, deps: Deps): Promise<void> {
  switch (action.kind) {
    case "move":
    case "archive":
      for (const [fromFolder, uids] of byFolder(action)) {
        await deps.bulkMove({ fromFolder, toFolder: action.folder!, uids });
      }
      return;
    case "mark_read":
      for (const [folder, uids] of byFolder(action)) {
        await deps.bulkFlags({ folder, uids, flags: ["\\Seen"], add: true });
      }
      return;
    case "create_filter":
      await deps.createFilter({
        name: `From ${action.filter!.from}`,
        conditions: [{ field: "from", op: "equals", value: action.filter!.from }],
        match_mode: "all",
        actions: [{ action_type: "move", action_value: action.filter!.folder }],
      });
      return;
  }
}

const card = "border border-border bg-card p-3 text-sm";

export function DraftCard({ draft, accountId }: { draft: DraftProposal; accountId: string | null }) {
  const queryClient = useQueryClient();
  const { data: identities } = useIdentities();
  const [state, setState] = useState<"idle" | "loading" | { error: string }>("idle");
  const stale = useIsStaleAccount(accountId);

  // Fetching the message body uses non-peek IMAP and marks it \Seen, so it
  // must only happen from the user's click — never as a side effect of the
  // card rendering.
  const openInCompose = async () => {
    if (stale) return;
    setState("loading");
    try {
      const data = await fetchMessage(queryClient, draft.folder, draft.uid);
      useComposeStore.getState().openReply(buildReplyParams(data, identities, draft.body));
      setState("idle");
    } catch (e) {
      setState({ error: e instanceof Error ? e.message : "Could not open this email" });
    }
  };

  return (
    <div className={card}>
      <p className="mb-2 text-xs font-medium uppercase tracking-wide text-muted-foreground">Draft reply</p>
      <p className="line-clamp-6 whitespace-pre-wrap">{draft.body}</p>
      <Button size="sm" className="mt-3" disabled={state === "loading" || stale} onClick={openInCompose}>
        {state === "loading" ? "Opening…" : "Open in compose"}
      </Button>
      {stale && <p className="mt-2 text-xs text-muted-foreground">From a different account; switch back to use it.</p>}
      {typeof state === "object" && <p role="alert" className="mt-2 text-xs text-destructive">{state.error}</p>}
    </div>
  );
}

export function ActionCard({ action, accountId }: { action: ActionProposal; accountId: string | null }) {
  const bulkMove = useBulkMoveMessages();
  const bulkFlags = useBulkUpdateFlags();
  const createFilter = useCreateFilter();
  const [state, setState] = useState<"idle" | "running" | "done" | "dismissed" | { error: string }>("idle");
  const stale = useIsStaleAccount(accountId);
  if (state === "dismissed") return null;
  const confirm = async () => {
    if (stale) return;
    setState("running");
    try {
      await runAction(action, {
        bulkMove: (a) => bulkMove.mutateAsync(a),
        bulkFlags: (a) => bulkFlags.mutateAsync(a),
        createFilter: (r) => createFilter.mutateAsync(r),
      });
      setState("done");
    } catch (e) {
      setState({ error: e instanceof Error ? e.message : "That did not work" });
    }
  };
  return (
    <div className={card}>
      <p className="font-medium">{action.summary}</p>
      {action.messages.length > 0 && (
        <ul className="mt-1 list-disc pl-4 text-xs text-muted-foreground">
          {action.messages.slice(0, 5).map((m) => <li key={`${m.folder}/${m.uid}`}>{m.subject || "(no subject)"}</li>)}
          {action.messages.length > 5 && <li>and {action.messages.length - 5} more</li>}
        </ul>
      )}
      {state === "done" ? (
        <p className="mt-2 text-xs text-primary">Done.</p>
      ) : (
        <div className="mt-3 flex gap-2">
          <Button size="sm" onClick={confirm} disabled={state === "running" || stale}>Confirm</Button>
          <Button size="sm" variant="ghost" onClick={() => setState("dismissed")} disabled={state === "running"}>Dismiss</Button>
        </div>
      )}
      {stale && <p className="mt-2 text-xs text-muted-foreground">From a different account; switch back to use it.</p>}
      {typeof state === "object" && <p role="alert" className="mt-2 text-xs text-destructive">{state.error}</p>}
    </div>
  );
}
