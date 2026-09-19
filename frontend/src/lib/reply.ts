import type { ReplyParams } from "@/stores/useComposeStore";
import type { EmailAddress, MessageDetail } from "@/types/message";
import type { Identity } from "@/types/identity";
import { buildReferences, buildReplyQuoteHtml, buildReplyQuoteText, buildReplySubject, extractHeader } from "@/lib/email-utils";

/** Find the identity whose email matches one of the To/CC addresses. */
export function findMatchingIdentity(
  identities: Identity[] | undefined,
  toAddresses: EmailAddress[],
  ccAddresses: EmailAddress[],
): number | null {
  if (!identities || identities.length === 0) return null;
  const allRecipientEmails = [...toAddresses, ...ccAddresses].map((a) =>
    a.address.toLowerCase(),
  );
  const match = identities.find((i) =>
    allRecipientEmails.includes(i.email.toLowerCase()),
  );
  return match?.id ?? null;
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/** Plain-text draft → compose HTML: blank lines split paragraphs, single newlines become <br>. */
export function draftToHtml(text: string): string {
  return text
    .trim()
    .split(/\n\s*\n/)
    .map((p) => `<p>${escapeHtml(p).replace(/\n/g, "<br>")}</p>`)
    .join("");
}

/**
 * The ReplyParams the Reply button builds, with an optional prefilled
 * plain-text draft body. The draft is converted to HTML only when the
 * original message was HTML (matching `isHtml`, which ComposeDialog uses to
 * pick a rich editor or a plain <textarea>) — otherwise it is used as-is.
 */
export function buildReplyParams(data: MessageDetail, identities: Identity[] | undefined, draftBody?: string): ReplyParams {
  const messageId = extractHeader(data.raw_headers, "Message-ID");
  const refs = extractHeader(data.raw_headers, "References");
  const hasHtml = !!(data.html && data.html.trim());
  const body = draftBody === undefined ? (hasHtml ? "<p><br></p>" : "") : hasHtml ? draftToHtml(draftBody) : draftBody;
  return {
    to: data.from_address,
    cc: "",
    subject: buildReplySubject(data.subject),
    body,
    quotedHtml: hasHtml ? buildReplyQuoteHtml(data.html!, data.from_address, data.date) : null,
    quotedText: buildReplyQuoteText(data.text, data.from_address, data.date),
    inReplyTo: messageId,
    references: buildReferences(refs, messageId),
    fromIdentityId: findMatchingIdentity(identities, data.to_addresses, data.cc_addresses),
    isHtml: hasHtml,
  };
}
