import { describe, expect, it } from "vitest";

import { resolveJunkFolderName } from "../folders";

type F = { name: string; attributes: string[] };
const f = (name: string, ...attributes: string[]): F => ({ name, attributes });

describe("resolveJunkFolderName", () => {
  it("prefers the folder the server flagged \\Junk, whatever it is called", () => {
    // mailcow/Dovecot: the folder is literally "Junk". Moving to "Spam"
    // targets a folder that does not exist, so the message is never filed and
    // mailcow's imapsieve rule - which fires on a copy into Junk - never runs.
    const folders = [f("INBOX"), f("Junk", "\\HasNoChildren", "\\Junk"), f("Trash", "\\Trash")];
    expect(resolveJunkFolderName(folders)).toBe("Junk");
  });

  it("matches the flag case-insensitively", () => {
    expect(resolveJunkFolderName([f("Junk", "\\junk")])).toBe("Junk");
  });

  it("falls back to a name match when no folder carries the flag", () => {
    expect(resolveJunkFolderName([f("INBOX"), f("Spam")])).toBe("Spam");
    expect(resolveJunkFolderName([f("INBOX"), f("Junk")])).toBe("Junk");
  });

  it("handles Gmail's nested spam folder", () => {
    expect(resolveJunkFolderName([f("INBOX"), f("[Gmail]/Spam")])).toBe("[Gmail]/Spam");
  });

  it("returns null when the mailbox has no junk folder at all", () => {
    // Better for the caller to disable the button than to invent a folder.
    expect(resolveJunkFolderName([f("INBOX"), f("Sent", "\\Sent")])).toBeNull();
  });

  it("prefers the flagged folder even when a differently-named one exists", () => {
    const folders = [f("Spam"), f("Junk", "\\Junk")];
    expect(resolveJunkFolderName(folders)).toBe("Junk");
  });
});
