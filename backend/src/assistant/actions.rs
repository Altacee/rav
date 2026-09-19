use super::events::{ActionKind, ActionMessage, ActionProposal, FilterSpec};
use serde::Deserialize;

pub const MAX_ACTION_MESSAGES: usize = 50;

/// Arguments of the `propose_action` tool, as the model sends them.
#[derive(Debug, Clone, Deserialize)]
pub struct ProposeActionArgs {
    pub kind: String,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
}

fn emails(n: usize) -> String {
    if n == 1 { "1 email".to_string() } else { format!("{n} emails") }
}

fn existing_folder(folders: &[String], name: &str) -> Result<String, String> {
    folders
        .iter()
        .find(|f| f.as_str() == name)
        .cloned()
        .ok_or_else(|| format!("folder \"{name}\" does not exist; use one of the user's folders"))
}

/// One exact address: no wildcards, no spaces, exactly one '@' with text on both sides.
fn exact_address(raw: &str) -> Result<String, String> {
    let a = raw.trim().to_lowercase();
    let mut parts = a.split('@');
    let ok = matches!((parts.next(), parts.next(), parts.next()), (Some(l), Some(d), None) if !l.is_empty() && d.contains('.'))
        && !a.chars().any(|c| c.is_whitespace() || matches!(c, '*' | '?' | '%'));
    if ok { Ok(a) } else { Err(format!("\"{raw}\" is not one exact sender address")) }
}

/// Turn a model's proposal into something the UI may show. Nothing here runs it.
pub fn validate_action(
    args: &ProposeActionArgs,
    messages: Vec<ActionMessage>,
    folders: &[String],
) -> Result<ActionProposal, String> {
    let kind = match args.kind.as_str() {
        "move" => ActionKind::Move,
        "archive" => ActionKind::Archive,
        "mark_read" => ActionKind::MarkRead,
        "create_filter" => ActionKind::CreateFilter,
        other => return Err(format!("\"{other}\" is not an action; use move, archive, mark_read or create_filter")),
    };
    if messages.len() > MAX_ACTION_MESSAGES {
        return Err(format!("at most {MAX_ACTION_MESSAGES} emails per action"));
    }
    if kind != ActionKind::CreateFilter && messages.is_empty() {
        return Err("name at least one email by its ref".to_string());
    }
    let n = messages.len();
    let (folder, filter, summary) = match kind {
        ActionKind::Move => {
            let f = existing_folder(folders, args.folder.as_deref().unwrap_or(""))?;
            let s = format!("Move {} to {f}", emails(n));
            (Some(f), None, s)
        }
        ActionKind::Archive => {
            let f = folders
                .iter()
                .find(|f| f.eq_ignore_ascii_case("archive"))
                .cloned()
                .ok_or("this mailbox has no Archive folder")?;
            (Some(f), None, format!("Archive {}", emails(n)))
        }
        ActionKind::MarkRead => (None, None, format!("Mark {} as read", emails(n))),
        ActionKind::CreateFilter => {
            let from = exact_address(args.from.as_deref().unwrap_or(""))?;
            let f = existing_folder(folders, args.folder.as_deref().unwrap_or(""))?;
            let s = format!("File future mail from {from} into {f}");
            (None, Some(FilterSpec { from, folder: f }), s)
        }
    };
    Ok(ActionProposal { id: uuid::Uuid::new_v4().to_string(), kind, summary, messages, folder, filter })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folders() -> Vec<String> {
        ["INBOX", "Archive", "Finance", "Junk"].map(String::from).to_vec()
    }
    fn msgs(n: usize) -> Vec<ActionMessage> {
        (1..=n).map(|i| ActionMessage { folder: "INBOX".into(), uid: i as u32, subject: format!("s{i}") }).collect()
    }
    fn args(kind: &str, folder: Option<&str>, from: Option<&str>) -> ProposeActionArgs {
        ProposeActionArgs { kind: kind.into(), refs: vec![], folder: folder.map(String::from), from: from.map(String::from) }
    }

    #[test]
    fn move_to_an_existing_folder() {
        let a = validate_action(&args("move", Some("Finance"), None), msgs(3), &folders()).unwrap();
        assert_eq!(a.kind, ActionKind::Move);
        assert_eq!(a.folder.as_deref(), Some("Finance"));
        assert_eq!(a.summary, "Move 3 emails to Finance");
    }

    #[test]
    fn move_to_a_made_up_folder_is_rejected() {
        let e = validate_action(&args("move", Some("Trash2"), None), msgs(1), &folders()).unwrap_err();
        assert!(e.contains("Trash2"), "{e}");
    }

    #[test]
    fn archive_resolves_the_archive_folder() {
        let a = validate_action(&args("archive", None, None), msgs(1), &folders()).unwrap();
        assert_eq!(a.folder.as_deref(), Some("Archive"));
        assert_eq!(a.summary, "Archive 1 email");
    }

    #[test]
    fn archive_without_an_archive_folder_is_rejected() {
        assert!(validate_action(&args("archive", None, None), msgs(1), &["INBOX".to_string()]).is_err());
    }

    #[test]
    fn mark_read_needs_messages() {
        assert_eq!(validate_action(&args("mark_read", None, None), msgs(2), &folders()).unwrap().summary, "Mark 2 emails as read");
        assert!(validate_action(&args("mark_read", None, None), vec![], &folders()).is_err());
    }

    #[test]
    fn filters_take_one_exact_address() {
        let a = validate_action(&args("create_filter", Some("Finance"), Some("Billing@AWS.example")), vec![], &folders()).unwrap();
        assert_eq!(a.filter.as_ref().unwrap().from, "billing@aws.example");
        assert_eq!(a.summary, "File future mail from billing@aws.example into Finance");
        for bad in ["*@aws.example", "no-reply", "a@b c", "a@@b", ""] {
            assert!(validate_action(&args("create_filter", Some("Finance"), Some(bad)), vec![], &folders()).is_err(), "{bad}");
        }
    }

    #[test]
    fn unknown_kinds_and_huge_batches_are_rejected() {
        assert!(validate_action(&args("delete", None, None), msgs(1), &folders()).is_err());
        assert!(validate_action(&args("send", None, None), msgs(1), &folders()).is_err());
        assert!(validate_action(&args("move", Some("Finance"), None), msgs(MAX_ACTION_MESSAGES + 1), &folders()).is_err());
    }
}
