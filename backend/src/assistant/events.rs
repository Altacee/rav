use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceRef {
    #[serde(rename = "ref")]
    pub msg_ref: String,
    pub folder: String,
    pub uid: u32,
    pub subject: String,
    pub from: String,
    pub date: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DraftProposal {
    #[serde(rename = "ref")]
    pub msg_ref: String,
    pub folder: String,
    pub uid: u32,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionMessage {
    pub folder: String,
    pub uid: u32,
    pub subject: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FilterSpec {
    pub from: String,
    pub folder: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Move,
    Archive,
    MarkRead,
    CreateFilter,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ActionProposal {
    pub id: String,
    pub kind: ActionKind,
    pub summary: String,
    pub messages: Vec<ActionMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<FilterSpec>,
}

/// One SSE event on `POST /api/assistant/chat`.
#[derive(Debug, Clone, PartialEq)]
pub enum AssistantEvent {
    Status { text: String },
    Text { delta: String },
    Sources(Vec<SourceRef>),
    Draft(DraftProposal),
    Action(ActionProposal),
    Error { message: String, retryable: bool },
    Done { input_tokens: u64, output_tokens: u64 },
}

impl AssistantEvent {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Status { .. } => "status",
            Self::Text { .. } => "text",
            Self::Sources(_) => "sources",
            Self::Draft(_) => "draft",
            Self::Action(_) => "action",
            Self::Error { .. } => "error",
            Self::Done { .. } => "done",
        }
    }

    pub fn data_json(&self) -> String {
        use serde_json::json;
        match self {
            Self::Status { text } => json!({ "text": text }).to_string(),
            Self::Text { delta } => json!({ "delta": delta }).to_string(),
            // Struct field order matters here (e.g. `ref` before `folder`), so these
            // three go through serde_json::to_string directly rather than via
            // json!(), which sorts object keys.
            Self::Sources(s) => serde_json::to_string(s).expect("SourceRef always serializes"),
            Self::Draft(d) => serde_json::to_string(d).expect("DraftProposal always serializes"),
            Self::Action(a) => serde_json::to_string(a).expect("ActionProposal always serializes"),
            Self::Error { message, retryable } => {
                json!({ "message": message, "retryable": retryable }).to_string()
            }
            Self::Done { input_tokens, output_tokens } => {
                json!({ "input_tokens": input_tokens, "output_tokens": output_tokens }).to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_event_encodes_name_and_json() {
        let e = AssistantEvent::Text { delta: "Hi".into() };
        assert_eq!(e.name(), "text");
        assert_eq!(e.data_json(), r#"{"delta":"Hi"}"#);
    }

    #[test]
    fn sources_use_ref_as_the_key() {
        let e = AssistantEvent::Sources(vec![SourceRef {
            msg_ref: "m1".into(), folder: "INBOX".into(), uid: 5,
            subject: "S".into(), from: "a@b".into(), date: "d".into(),
        }]);
        assert_eq!(e.name(), "sources");
        assert!(e.data_json().starts_with(r#"[{"ref":"m1","folder":"INBOX","uid":5"#));
    }

    #[test]
    fn action_kind_is_snake_case() {
        let e = AssistantEvent::Action(ActionProposal {
            id: "x".into(), kind: ActionKind::MarkRead, summary: "s".into(),
            messages: vec![], folder: None, filter: None,
        });
        assert!(e.data_json().contains(r#""kind":"mark_read""#));
        assert!(!e.data_json().contains("folder"), "None fields are omitted");
    }
}
