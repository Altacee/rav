export interface AssistantSource {
  ref: string;
  folder: string;
  uid: number;
  subject: string;
  from: string;
  date: string;
}

export interface DraftProposal {
  ref: string;
  folder: string;
  uid: number;
  body: string;
}

export type ActionKind = "move" | "archive" | "mark_read" | "create_filter";

export interface ActionMessage {
  folder: string;
  uid: number;
  subject: string;
}

export interface ActionProposal {
  id: string;
  kind: ActionKind;
  summary: string;
  messages: ActionMessage[];
  folder?: string;
  filter?: { from: string; folder: string };
}

export type AssistantEvent =
  | { type: "status"; text: string }
  | { type: "text"; delta: string }
  | { type: "sources"; sources: AssistantSource[] }
  | { type: "draft"; draft: DraftProposal }
  | { type: "action"; action: ActionProposal }
  | { type: "error"; message: string; retryable: boolean }
  | { type: "done"; inputTokens: number; outputTokens: number };
