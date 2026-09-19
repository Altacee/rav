#!/usr/bin/env python3
"""Label new mail with TypeSafe (Jev): who sent it, what it is for, and how to treat it.

Each run labels every message in FOLDERS it has not labelled before, newest
first, up to MAX_PER_RUN, into a SQLite table. Designed for a CronJob.

    python3 triage.py sync       # label new mail (needs TYPESAFE_SEND_MAIL=yes)
    python3 triage.py selftest   # offline checks, no network

One request per message asks three Choice questions. Answers are always one of
the listed options and carry probabilities and a confidence, so there is no
JSON repair or retry. Rows below GATE confidence are marked for a second opinion:
in the 2026-09-19 eval the one genuine lead (relayed through no-reply@) won only
at p=0.47, and a local model caught it.

Mail bodies go to api.typesafe.ai - the mailbox owner approved that for this
mailbox. They are never logged or stored here; only labels and numbers are.
"""
import email, imaplib, json, os, re, sqlite3, sys, time
import urllib.error, urllib.request
from email import policy as pol

API = "https://api.typesafe.ai/v1/systemone"
# Pinned, not jev-latest: an alias can move and shift what GATE means.
MODEL = os.environ.get("TYPESAFE_MODEL", "jev-1.13.0")
IMAP_HOST = os.environ.get("IMAP_HOST", "ryuvzdff.altacee.com")
FOLDERS = os.environ.get("FOLDERS", "INBOX,Junk").split(",")
DB = os.environ.get("DB", "/data/triage.sqlite")
MAX_PER_RUN = int(os.environ.get("MAX_PER_RUN", "300"))
GATE = float(os.environ.get("GATE", "0.6"))
CAP_CHARS = 6000  # ~1500 tokens; the eval found the cap almost never binds

QUESTIONS = {
    "relationship": {
        "type": "choice",
        "instructions": "Who is the sender of `email` to the mailbox owner (`mailbox.owner`)?",
        "criteria": {
            "internal": "Works in the same organisation as the owner (same domain as `mailbox.organisation_domain`) and wrote this personally.",
            "client": "Buys, or wants to buy, from the owner's organisation - including an enquiry relayed through a website contact form.",
            "vendor": "The owner's organisation buys from them: a supplier, service provider or platform billing the owner.",
            "automated": "No person wrote it: alerts, notifications, receipts, newsletters, system mail.",
            "personal": "A friend or family member writing about private life.",
            "unknown": "A stranger with no established relationship, including spammers and cold outreach.",
        },
    },
    "intent": {
        "type": "choice",
        "instructions": "What is `email` for?",
        "criteria": {
            "request": "Asks the owner to do, decide, answer or review something.",
            "contract": "An agreement, proposal, quote terms or legal document.",
            "invoice": "A bill, payment request, receipt or payment confirmation.",
            "report": "Status, monitoring alerts, analytics or a summary of activity.",
            "security": "Sign-in codes, password resets, security warnings about an account the owner holds.",
            "promotion": "Marketing, offers, newsletters, or anything selling something.",
            "conversation": "Ongoing back-and-forth between people with no specific ask.",
            "other": "None of the above.",
        },
    },
    "priority": {
        "type": "choice",
        "instructions": "How should the mailbox owner treat `email`? This is about whether the owner owes someone a response.",
        "criteria": {
            "needs_reply": "A real person who knows the owner, or a genuine prospective client, is waiting on the owner: a question, a decision, a document to review, a thread the owner is part of.",
            "fyi": "Worth seeing but needs nothing: receipts, confirmations, reports, alerts about accounts the owner holds.",
            "noise": "Bulk, promotional or unsolicited mail, including anything from an unknown or automated sender urging the reader to click, pay, verify, claim or act urgently. Spam is always noise however urgent it sounds - the urgency is the tell, not the reason.",
        },
    },
}

QUOTE = re.compile(r"^\s*>.*$", re.M)
TAGS = re.compile(r"<[^>]+>")
WS = re.compile(r"[ \t]+")


def readable(msg):
    """Plain text of the message without quoted replies; HTML tags dropped."""
    body = msg.get_body(preferencelist=("plain", "html"))
    if body is None:
        return ""
    try:
        text = body.get_content()
    except Exception:
        return ""
    if body.get_content_type() == "text/html":
        text = TAGS.sub(" ", text)
    return WS.sub(" ", QUOTE.sub("", text)).strip()


def build_state(owner, sender, to, subject, body):
    return {
        "mailbox": {"owner": owner, "organisation_domain": owner.rsplit("@", 1)[-1]},
        "email": {"from": sender, "to": to, "subject": subject, "body": body[:CAP_CHARS]},
    }


def parse_answers(resp):
    """Flatten a /v1/systemone response into one row. Raises on anything off-contract."""
    answers = resp["answers"]
    row = {"answered_by": resp.get("model", ""),
           "input_tokens": resp.get("usage", {}).get("input_tokens", 0),
           "probs": {}}
    for q, spec in QUESTIONS.items():
        choice = answers[q]["choice"]
        if choice not in spec["criteria"]:
            raise ValueError(f"{q}: {choice!r} is not an option")
        row[q] = choice
        row[q + "_conf"] = float(answers[q]["confidence"])
        row["probs"][q] = answers[q]["probabilities"]
    return row


def ask(state, key):
    body = json.dumps({"model": MODEL, "state": state, "questions": QUESTIONS}).encode()
    req = urllib.request.Request(API, data=body, headers={
        "Authorization": "Bearer " + key, "Content-Type": "application/json"})
    for attempt in range(4):
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                return parse_answers(json.loads(r.read()))
        except urllib.error.HTTPError as e:
            if (e.code != 429 and e.code < 500) or attempt == 3:
                raise
            time.sleep(float(e.headers.get("retry-after") or 2 ** attempt))


def open_db(path):
    db = sqlite3.connect(path)
    db.execute("PRAGMA journal_mode=WAL")
    # ponytail: keyed by (folder, uid) without UIDVALIDITY; if mailcow ever
    # resets a folder's UIDs, delete that folder's rows and let it relabel.
    db.execute("""CREATE TABLE IF NOT EXISTS triage(
        folder TEXT, uid INTEGER, model TEXT, answered_by TEXT,
        subject TEXT, sender TEXT, received TEXT,
        relationship TEXT, relationship_conf REAL, intent TEXT, intent_conf REAL,
        priority TEXT, priority_conf REAL, needs_second_opinion INTEGER,
        probs_json TEXT, chars INTEGER, input_tokens INTEGER, seconds REAL, labelled_at TEXT,
        UNIQUE(folder, uid, model))""")
    return db


def label_folder(imap, db, key, owner, folder, budget):
    """Label up to `budget` unlabelled messages in one folder, newest first. Returns count."""
    typ, _ = imap.select('"%s"' % folder, readonly=True)
    if typ != "OK":
        print(f"{folder}: cannot select, skipped", flush=True)
        return 0
    typ, data = imap.uid("SEARCH", None, "ALL")
    uids = [int(u) for u in (data[0].split() if typ == "OK" and data and data[0] else [])]
    have = {r[0] for r in db.execute(
        "SELECT uid FROM triage WHERE folder=? AND model=?", (folder, MODEL))}
    todo = sorted(set(uids) - have, reverse=True)[:budget]
    print(f"{folder}: {len(uids)} messages, {len(todo)} to label this run", flush=True)
    done = 0
    for uid in todo:
        typ, r = imap.uid("FETCH", str(uid), "(BODY.PEEK[])")  # PEEK: never marks mail read
        if typ != "OK" or not r or not isinstance(r[0], tuple):
            print(f"  uid {uid}: not fetched", flush=True)
            continue
        msg = email.message_from_bytes(r[0][1], policy=pol.default)
        text = readable(msg)
        sender, subject = str(msg.get("From", "")), str(msg.get("Subject", ""))
        t0 = time.time()
        try:
            row = ask(build_state(owner, sender, str(msg.get("To", "")), subject, text), key)
        except Exception as e:  # the error type only: never echo mail content
            print(f"  uid {uid} failed: {type(e).__name__} {getattr(e, 'code', '')}", flush=True)
            continue
        db.execute("INSERT OR REPLACE INTO triage VALUES "
                   "(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,datetime('now'))",
                   (folder, uid, MODEL, row["answered_by"],
                    subject[:200], sender[:200], str(msg.get("Date", ""))[:64],
                    row["relationship"], row["relationship_conf"], row["intent"], row["intent_conf"],
                    row["priority"], row["priority_conf"], int(row["priority_conf"] < GATE),
                    json.dumps(row["probs"]), min(len(text), CAP_CHARS), row["input_tokens"],
                    round(time.time() - t0, 2)))
        db.commit()
        done += 1
    return done


def sync():
    if os.environ.get("TYPESAFE_SEND_MAIL") != "yes":
        sys.exit("refusing: this sends mail bodies to api.typesafe.ai; "
                 "set TYPESAFE_SEND_MAIL=yes only with the mailbox owner's approval")
    key, owner = os.environ["TYPESAFE_API_KEY"].strip(), os.environ["IMAP_USER"]
    db = open_db(DB)
    imap = imaplib.IMAP4_SSL(IMAP_HOST, 993)
    imap.login(owner, os.environ["IMAP_PASS"])
    budget = MAX_PER_RUN
    try:
        for folder in FOLDERS:
            budget -= label_folder(imap, db, key, owner, folder, budget)
            if budget <= 0:
                break
    finally:
        imap.logout()
    total, low, toks = db.execute(
        "SELECT count(*), sum(needs_second_opinion), sum(input_tokens) FROM triage WHERE model=?",
        (MODEL,)).fetchone()
    print(f"labelled {MAX_PER_RUN - budget} this run; {total} total, {low or 0} below "
          f"the {GATE} gate, {toks or 0:,} input tokens so far", flush=True)


FAKE_RESPONSE = {
    "model": "jev-1.13.0",
    "answers": {
        "relationship": {"type": "choice", "choice": "unknown", "confidence": 0.9,
                         "probabilities": {"unknown": 0.95, "automated": 0.05}},
        "intent": {"type": "choice", "choice": "promotion", "confidence": 0.8,
                   "probabilities": {"promotion": 0.9, "security": 0.1}},
        "priority": {"type": "choice", "choice": "noise", "confidence": 0.97,
                     "probabilities": {"noise": 0.98, "fyi": 0.02}},
    },
    "usage": {"input_tokens": 812, "output_tokens": 30},
}


def selftest():
    state = build_state("owner@altacee.dev", "Win <prize@x.example>", "owner@altacee.dev",
                        "Claim now", "x" * (CAP_CHARS + 50))
    assert state["mailbox"]["organisation_domain"] == "altacee.dev"
    assert len(state["email"]["body"]) == CAP_CHARS
    assert list(QUESTIONS["priority"]["criteria"]) == ["needs_reply", "fyi", "noise"]
    row = parse_answers(FAKE_RESPONSE)
    assert (row["priority"], row["priority_conf"], row["input_tokens"]) == ("noise", 0.97, 812)
    bad = json.loads(json.dumps(FAKE_RESPONSE))
    bad["answers"]["priority"]["choice"] = "Needs_Reply"
    try:
        parse_answers(bad)
        raise AssertionError("an off-list option must be rejected")
    except ValueError:
        pass
    quoted = email.message_from_string(
        "Content-Type: text/plain\n\nThanks, will do.\n> old quoted line\n", policy=pol.default)
    assert readable(quoted) == "Thanks, will do."
    db = open_db(":memory:")
    assert db.execute("SELECT count(*) FROM triage").fetchone() == (0,)
    print("selftest ok")


if __name__ == "__main__":
    cmds = {"sync": sync, "selftest": selftest}
    if len(sys.argv) != 2 or sys.argv[1] not in cmds:
        sys.exit("usage: triage.py " + "|".join(cmds))
    cmds[sys.argv[1]]()
