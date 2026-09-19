import { describe, expect, it } from "vitest";
import { buildReplyParams, draftToHtml } from "@/lib/reply";
import type { MessageDetail } from "@/types/message";

const detail = {
  uid: 4, folder_id: "x", folder_name: "INBOX", subject: "Contract", from_address: "anu@altacee.com", from_name: "Anu",
  to_addresses: [{ name: null, address: "me@altacee.dev" }], cc_addresses: [], date: "2026-09-17", flags: [],
  has_attachments: false, html: "<p>Please sign</p>", text: "Please sign",
  raw_headers: "Message-ID: <abc@x>\r\nReferences: <root@x>\r\n", attachments: [], thread: [], email_theme: null, pgp_status: null,
} as unknown as MessageDetail;

describe("draftToHtml", () => {
  it("escapes and turns paragraphs into <p>", () => {
    expect(draftToHtml("Hi <Anu>,\n\nSigned & sent.\nThanks")).toBe("<p>Hi &lt;Anu&gt;,</p><p>Signed &amp; sent.<br>Thanks</p>");
  });
});

describe("buildReplyParams", () => {
  it("converts a plain-text draft to HTML when the original was HTML", () => {
    const p = buildReplyParams(detail, [{ id: 7, email: "me@altacee.dev" }] as never, "Done");
    expect(p.to).toBe("anu@altacee.com");
    expect(p.subject).toMatch(/^Re: Contract/);
    expect(p.inReplyTo).toBe("<abc@x>");
    expect(p.references).toContain("<root@x>");
    expect(p.body).toBe("<p>Done</p>");
    expect(p.fromIdentityId).toBe(7);
    expect(p.isHtml).toBe(true);
  });

  it("keeps a plain-text draft as plain text when the original had no HTML", () => {
    const plainDetail = { ...detail, html: null };
    const p = buildReplyParams(plainDetail, undefined, "Signed, thanks.");
    expect(p.body).toBe("Signed, thanks.");
    expect(p.isHtml).toBe(false);
  });
});
