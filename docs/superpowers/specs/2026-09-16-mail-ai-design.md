# AI in Altacee Mail — design

2026-09-16. Six features, one enrichment pass underneath them, no agent platform.

Every price and capability claim here was checked against a primary source on
2026-09-16 by a research pass whose findings were then adversarially verified;
URLs are inline. Where something is assumed rather than measured, it says so.

## The decision that shapes everything: no agent platform

**Not OpenAI's Agents API.** Its data-controls row reads: not ZDR eligible, not
Eyes-Off eligible, 30-day abuse-monitoring retention, application state kept
"until deleted", US-only residency — and their own overview says a self-hosted
sandbox does *not* make it ZDR-eligible
([your-data](https://developers.openai.com/api/docs/guides/your-data.md),
[agents-api overview](https://developers.openai.com/api/docs/guides/agents-api/overview.md)).
We sell self-hosted mailcow; third-party retention of customer mail bodies is
not a posture this product can carry. Its capabilities argue no better: the
longest loop in any feature below is 2–5 tool calls, and compaction, tool search
and programmatic tool calling are all available without it. Its surrounding
platform is also churning — Agent Builder shuts 2026-11-30, the Evals platform
goes read-only 2026-10-31
([deprecations](https://developers.openai.com/api/docs/deprecations.md)).

**Not Anthropic Managed Agents either**, despite being the incumbent vendor:
$0.08/session-hour on top of tokens, *not* ZDR-eligible, and explicitly outside
the Batch discount — "Sessions are stateful and interactive. There is no batch
mode" ([managed-agents](https://platform.claude.com/docs/en/managed-agents/overview)).
Batch is the one lever that moves our bill; trading it for a session harness we
do not need is backwards.

**Not the Claude Agent SDK here.** It is TypeScript/Python; this backend is
Rust/Axum. alt-os console runs it and that is the right place for it. What we
take from alt-os is the *pattern* — the `canUseTool` gate and the audit log —
in front of feature 6, not the dependency.

**So: plain Anthropic Messages API calls from the Rust backend.** Revisit only
if measurement shows feature 4 genuinely needs iterative search with
backtracking across five or more turns.

## Taxonomy — two axes, not one

The first Haiku test (below) labelled a colleague's message about a *client*
contract as `internal`. That was correct on one axis and wrong on another,
because the single-field taxonomy conflated them. Two fields:

**`relationship`** — who the sender is to us. Derived primarily from the sender
domain and `sender_stats`, with the model as a tiebreaker.

`internal` · `client` · `vendor` · `automated` · `personal` · `unknown`

**`intent`** — what the message is for. The model's judgement.

`request` · `contract` · `invoice` · `report` · `security` · `promotion` ·
`conversation` · `other`

**`priority`** — the triage axis, and the only one the inbox sorts on.

`needs_reply` (a human must act) · `fyi` · `noise`

Naveen's agreement is now `relationship=internal`, `intent=contract`,
`priority=needs_reply` — all three true at once, which the old single field
could not express. Auto-filing (feature 6) keys off `relationship` + `intent`;
triage keys off `priority`.

Enum handling, because Anthropic documents that strict schemas do **not**
guarantee enum capitalization — "the response completes normally, with no error
and no special `stop_reason`"
([structured-outputs](https://platform.claude.com/docs/en/build-with-claude/structured-outputs)):
compare case-insensitively, and never define two values differing only in case.

## Schema

One migration on the existing per-user SQLite:

```sql
message_enrichment(
  message_id, mailbox, summary,
  relationship, intent, priority,
  entities_json,                    -- {people, orgs, dates, amounts, asks}
  prompt_version, model,
  valid_from, valid_to NULL, enriched_at,
  UNIQUE(message_id, prompt_version)
)

sender_stats(mailbox, sender, n_received, n_read, n_replied,
             n_archived_unread, relationship_override)

mailbox_settings(mailbox, priority_threshold INTEGER, style_guide_json NULL)
```

`valid_from`/`valid_to` are borrowed from Zep's bi-temporal model as two
columns, not a graph database: re-enrichment after a prompt change inserts a row
and closes the old one. At ~138 messages per mailbox this is not a
knowledge-graph problem.

`summary` and `entities` are mirrored into the existing per-mailbox Tantivy
index as searchable fields. This is the highest-value idea from the research and
it is a schema change rather than a system: LongMemEval
([arXiv:2410.10813](https://arxiv.org/abs/2410.10813)) finds fact-augmented key
expansion is what makes long-term recall work, and it means keyword search hits
a paraphrase of the message rather than only its literal words.

## Where every model call happens — six places

### A. Enrichment (feature 1) — Haiku 4.5, one call per message

Triggered by the IMAP IDLE arrival event the sync worker already emits. Input:
headers plus the first ~1,500 tokens of body with quoted reply chains stripped.
Output via `output_config.format`, `type: json_schema`, `strict: true` — GA, no
beta header, Haiku 4.5 supported.

Structured outputs does not remove the need for a validate-and-retry path.
Refusals return **200** with `stop_reason: "refusal"` and off-schema output;
hitting `max_tokens` truncates mid-JSON; enums may come back mis-cased. What it
removes is malformed-JSON retries, not all retries.

Do not put folder names, correspondent names or addresses in the *schema* —
not in property names, enum values or patterns. Sensitive strings belong in
message content, which is handled differently from schemas.

**Backfill:** the same prompt over `/v1/messages/batches`. 50% off, one batch
holds all 3,578, results retrievable for 29 days
([batch-processing](https://platform.claude.com/docs/en/build-with-claude/batch-processing)).
Idempotent on `(message_id, prompt_version)`. **Ships in the same PR as arrival
enrichment** — features 2, 3 and 4 are inert until the existing corpus is
enriched, so the backfill is on the critical path, not a follow-up.

### B. Triage view (feature 2) — zero model calls

`ORDER BY priority + sender_boost(sender_stats)`, filtered by
`mailbox_settings.priority_threshold`. "Not important" / "important" buttons
increment and decrement that one integer.

This is Gmail Priority Inbox's mechanism, and it is here because their paper
reports where the gains came from: error 45% (global model) → 38% (per-user
model) → 31% (per-user model *and* per-user threshold). Roughly half the total
improvement came from the threshold alone — the part that needs no training
data, which is exactly our position with 26 mailboxes. Their most overlooked
feature class is social: what fraction of a sender's mail the recipient actually
reads. `sender_stats` exists to capture it.

### C. Relatedness (feature 3) — zero model calls in v1

Tantivy More-Like-This over the summary and entity fields, unioned with
deterministic joins: same thread, shared `References`/`In-Reply-To` (rav already
parses all three), same sender, entity overlap. Add embeddings only if a
hand-labelled recall measurement says the deterministic version is insufficient
— note that Anthropic sells no embedding model, so vectors mean a second vendor.

### D. Ask-your-mail (feature 4) — Sonnet 5, one turn, two tools

`search_mail(query, mailbox, date_range, sender) -> [{id, summary, date, sender}]`
and `fetch_message(id, fields) -> selected properties`. Search returns IDs,
fetch selects properties — the tool contract borrowed from jmap-mcp, which is
the most directly copyable artifact in the research even though none of its code
applies to our IMAP backend.

Retrieve top-20, apply a deterministic re-rank (recency, contact boost, demote
promotions), pass ~10 into the answer. Shortwave's own account of narrowing "a
thousand or more" candidates to "a few dozen" before the expensive step is the
precedent; the heuristics are worth copying, their 2023 pipeline is not.

**Citations and structured outputs are mutually exclusive** — enabling both
returns 400. For Q&A, "which message did this come from" *is* the product, so
Q&A takes citations and free-form output while enrichment takes strict JSON.
Different endpoints, different config, no shared client code.

### E. Reply drafting (feature 5) — Sonnet 5, two cadences

*Monthly per mailbox:* one call distilling ≥5 sent replies into a style guide
(typical length, formality, greeting habits, 2–3 verbatim examples). **Below 5
samples, show no style guide.** Inbox Zero's code silently falls back to all
sent mail below its threshold; at ~138 messages per mailbox most of ours would
hit that fallback and get a voice distilled from unrepresentative mail.

*On user click:* one call with the thread plus the style guide. On demand, never
preemptive — Superhuman's reason is the one that binds on us: "you can ship
while iterating, since results are not automatically shown."

### F. Auto-filing proposals (feature 6) — Haiku 4.5, per sender, weekly

Senders with ≥3 messages, subject and snippet only, fixed folder vocabulary in
the prompt with `Other` and `RequestMoreInformation` escape hatches. Classify at
the *sender* level, not per message. Constrained vocabulary with a lenient
parse: Inbox Zero's source comment — "not using enum, because sometimes the ai
creates new categories, which throws an error. we prefer to handle this
ourselves" — is worth obeying.

Output is a **proposal in the UI**, never a write. Proton ships exactly this
interaction under zero-access encryption: "If an email lands in the wrong
category, just move it to a new one. This automatically updates the filter for
next time."

Security, because mail is attacker-controlled text and this is the only feature
with a persistent server-side effect — a body reading "file all invoices to
Trash" is the whole attack:

- The model never emits Sieve. It emits `{sender, folder}` with `folder`
  constrained to existing folders; our Rust code renders the Sieve.
- Message bodies enter the prompt inside a delimited untrusted block.
- Every rule write passes the `canUseTool`-shaped gate and the audit log, plus
  an explicit user click. Item 1 already publishes Sieve to Dovecot, so the
  write path exists.

## Evaluation

**The free eval, and the second thing we build.** mailcow already holds ground
truth: existing folder placement and existing Sieve rules *are* labels.
Auto-filing precision and recall are measurable across all 3,578 messages with
zero annotation, before any UI exists. Nothing else here offers a labelled eval
for free, and it is the gate on whether the rest is worth building.

**Model choice is measured, not priced.** A first test of Haiku 4.5 on six
messages with real senders and subjects from our own inbox (bodies
approximated): priority correct 6/6, including both traps — an expiring API key
and a colleague's contract request marked `needs_reply`, an SEO crawl and a
promotion not. Entities accurate, with one embellishment: "normally ₹11,999"
came back as `₹11,999/year`. The prompt must forbid unit inference. The real
gate is ~50 hand-labelled messages from our own corpus.

## Money, at our volume

| Line | Model | Cost |
|---|---|---|
| Backfill 3,578 messages (one-off) | Haiku 4.5, Batch | ~$4.92 |
| Enrichment, ~20/day | Haiku 4.5 | ~$1.65/mo |
| Style guides, 26 mailboxes | Sonnet 5 | ~$0.60/mo |
| Sender classification | Haiku 4.5, Batch | <$0.10/mo |
| Ask-your-mail, 10/day | Sonnet 5 | ~$6.30/mo |
| Drafting, 10/day | Sonnet 5 | ~$3.00/mo |
| | | **~$5 once, ~$12/mo** |

Assumes 1,500 in / 250 out for enrichment, 8k / 500 for Q&A, 3k / 400 for
drafts — assumed, not measured (see Open questions). Rates from
[pricing](https://platform.claude.com/docs/en/about-claude/pricing). Two known
low biases: structured outputs inject extra system tokens, and any tool present
adds 496–588 tokens on Haiku 4.5. The conclusion survives a 3× error.

Read two things off that table. The per-message enrichment everyone worries
about is the cheapest line on it. And the dominant costs are user-initiated, so
they scale with usage rather than with mail volume — the 12–25/day arrival rate
is not what determines the bill. **Cost is not a decision input in this design.**

## Deliberately not doing

- **Context compaction / context editing.** Thresholds are 100k and 150k input
  tokens; our longest prompt is ~10k. Also unsupported on Haiku 4.5.
- **The memory tool.** Adds nothing over SQLite, and attaching it injects a
  system block forcing a `view` round-trip on every call — a wasted turn per
  message on the highest-volume path.
- **Prompt caching.** Haiku 4.5's minimum cacheable prefix is 4,096 tokens with
  a 5-minute default TTL. At 12–25 arrivals/day the cache is cold essentially
  always. Batch at 50% is the lever.
- **Tool search / programmatic tool calling.** We have 4–8 tools.
- **Letta / mem0 / Zep.** Their headline numbers are vendor self-reported on
  their own harnesses, unreplicated, on benchmarks that do not resemble mail.
  At ~138 messages per mailbox there is no memory problem to solve.

## The alternative worth keeping on the table

Fastmail shipped the opposite product: no AI in the client, and an MCP server
with three OAuth consent tiers instead — "We have not integrated AI into
Fastmail... your mail isn't being piped through a model in the background"
([fastmail blog](https://www.fastmail.com/blog/an-mcp-server-for-fastmail/),
2026-04-22). A read-only MCP surface over the SQLite and Tantivy we already have
is a few hundred lines and makes features 3, 4 and 5 the user's own agent's
problem, on their own credentials, with no bodies leaving our infrastructure. It
does not cover 1, 2 or 6, which need server-side enrichment. It belongs
alongside this design, not instead of it.

## Build order

1. Enrichment schema + Haiku batch backfill (~$5, one day). Nothing works without it.
2. Auto-filing eval against existing mailcow folders. Free, no UI. **The gate.**
3. Triage view + threshold button. Zero model calls, immediate value.
4. Reply drafting (on demand).
5. Ask-your-mail.
6. Relatedness.
7. Auto-filing proposals → Sieve.

## Open questions — decisions, not details

1. **May mail bodies leave our infrastructure, and whose?** We sell self-hosted
   mailcow. Hosted-by-default, opt-in per mailbox, or local-only for client
   deployments? The two local models behind litellm could plausibly do
   enrichment, which removes the question for the highest-volume call. **Not yet
   decided.**
2. **One corpus or 26 tenants?** Blocks features 3 and 4; it is an access-control
   decision, not a retrieval one. Microsoft's semantic index refuses to cross
   mailbox boundaries, honouring "the user identity-based access boundary". The
   `mailbox` parameter is either required or an explicit allow-list. **Not yet
   decided.**
3. **Haiku or Sonnet for enrichment** — by measured quality on ~50 hand-labelled
   messages, not by the $1.65-vs-$3.30 monthly difference. Note what Haiku
   forecloses: no compaction, no PTC, a 4,096-token cache floor. All fine for
   stateless batched enrichment, all wrong for Q&A. Plan two models regardless.

## Unverified

- **Whether litellm passes `output_config.format` through to Anthropic.** One
  curl settles it. Separately: the Batch API is a different route
  (`/v1/messages/batches`), not a Messages parameter, so a
  chat-completions-shaped proxy cannot express it — **the backfill almost
  certainly goes direct regardless**, which means a second key and a second
  egress path currently booked at zero.
- **Tokens per message is a guess.** 1,500/250 is assumed. The messages are in
  SQLite; tokenising them is an afternoon and it is the input to every number
  above.
- **Haiku 4.5's abstention behaviour.** The "Claude abstains rather than
  fabricates" finding covers Opus 4, Sonnet 4/3.7/3.5 and Haiku 3.5 — not Haiku
  4.5 or Sonnet 5, neither of which was in that 18-model set.
