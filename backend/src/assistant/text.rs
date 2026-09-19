use regex::Regex;
use std::sync::LazyLock;

/// Body characters sent to the model for one message.
pub const BODY_CAP: usize = 6000;
/// Per-message cap inside a thread.
pub const THREAD_CAP: usize = 2000;

static DROP_BLOCKS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?is)<(style|script|head)\b[^>]*>.*?</(style|script|head)>").unwrap());
static BREAKS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)<br\s*/?>|</(p|div|li|tr|h[1-6])>").unwrap());
static TAGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]+>").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[ \t\u{a0}]+").unwrap());

fn html_to_text(html: &str) -> String {
    let s = DROP_BLOCKS.replace_all(html, "");
    let s = BREAKS.replace_all(&s, "\n");
    let s = TAGS.replace_all(&s, "");
    s.replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// The part of a message a reader would read: plain text if present, else the
/// HTML's text; quoted reply lines and blank lines dropped; capped at `cap`
/// characters with a visible marker.
pub fn readable_body(html: Option<&str>, text: Option<&str>, cap: usize) -> String {
    let raw = match (text.map(str::trim).filter(|t| !t.is_empty()), html) {
        (Some(t), _) => t.to_string(),
        (None, Some(h)) => html_to_text(h),
        (None, None) => String::new(),
    };
    let body = raw
        .lines()
        .map(|l| SPACES.replace_all(l, " ").trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('>'))
        .collect::<Vec<_>>()
        .join("\n");
    if body.chars().count() <= cap {
        return body;
    }
    let cut: String = body.chars().take(cap).collect();
    format!("{cut}…[truncated]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_plain_text_and_strips_quotes() {
        let body = readable_body(Some("<p>html</p>"), Some("Thanks!\n> old\n>> older\nBye"), BODY_CAP);
        assert_eq!(body, "Thanks!\nBye");
    }

    #[test]
    fn html_becomes_text_without_style_or_script() {
        let html = "<style>p{color:red}</style><p>Hello &amp; welcome</p><script>x()</script><div>Line&nbsp;two</div>";
        assert_eq!(readable_body(Some(html), None, BODY_CAP), "Hello & welcome\nLine two");
    }

    #[test]
    fn caps_on_a_char_boundary_and_says_so() {
        let body = readable_body(None, Some(&"é".repeat(50)), 10);
        assert_eq!(body, format!("{}…[truncated]", "é".repeat(10)));
    }

    #[test]
    fn empty_input_is_empty() {
        assert_eq!(readable_body(None, None, BODY_CAP), "");
        assert_eq!(readable_body(Some("   "), Some(""), BODY_CAP), "");
    }
}
