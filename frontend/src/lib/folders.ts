import type { QueryClient } from "@tanstack/react-query";
import type { FolderId } from "@/types/folder";
import type { FoldersResponse } from "@/types/folder";

/**
 * Resolve a folder name to its current FolderId by reading the warm
 * `["folders"]` query cache synchronously - no network request, safe to call
 * from inside a queryFn/mutationFn (unlike a hook, which cannot be called
 * outside render).
 *
 * FolderIds are single-use and non-deterministic (a fresh one is minted on
 * every server response), so this is the only correct way to go from a
 * folder *name* to a usable id. If you already have a record (Folder,
 * MessageHeader, MessageDetail, SearchResultItem) with its own `folder_id`,
 * use that directly instead - it's already valid and this lookup is
 * unnecessary.
 *
 * Throws if the folders list hasn't loaded yet or the name doesn't match any
 * known folder, rather than silently sending a request with "undefined" in
 * the URL.
 */
export function resolveFolderId(queryClient: QueryClient, name: string): FolderId {
  const folders = queryClient.getQueryData<FoldersResponse>(["folders"])?.folders;
  const id = folders?.find((f) => f.name === name)?.id;
  if (!id) {
    throw new Error(`Unknown folder "${name}" - folder list not loaded or folder does not exist`);
  }
  return id;
}

/** The names servers use for a junk folder when they do not flag it. */
const JUNK_NAMES = ["junk", "spam", "[gmail]/spam", "bulk mail"];

/**
 * The real name of this mailbox's junk folder, or null if it has none.
 *
 * Resolve by the server's `\Junk` special-use attribute first, because the
 * display name and the IMAP name are not the same thing: FolderTree shows
 * "Spam" for a folder mailcow actually calls "Junk". Moving to the display
 * name targets a folder that does not exist — the message is never filed, and
 * mailcow's imapsieve rule, which trains rspamd on a copy into `Junk`, never
 * fires.
 *
 * Returns null rather than guessing a name: a caller should disable the
 * action, not invent a folder.
 */
export function resolveJunkFolderName(
  folders: ReadonlyArray<{ name: string; attributes: string[] }> | undefined,
): string | null {
  if (!folders?.length) return null;
  const flagged = folders.find((f) =>
    f.attributes?.some((a) => a.toLowerCase() === "\\junk"),
  );
  if (flagged) return flagged.name;
  const named = folders.find((f) => JUNK_NAMES.includes(f.name.toLowerCase()));
  return named ? named.name : null;
}
