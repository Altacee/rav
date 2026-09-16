//! QRESYNC (RFC 7162) helpers.
//!
//! Pure string and range work, kept apart from the session so the parts worth
//! testing can be tested without an IMAP server.

/// Largest number of UIDs a single VANISHED range may expand to. A folder with
/// more deletions than this resyncs the slow way instead, which is correct and
/// bounded — unlike allocating whatever the server claimed.
const MAX_RANGE_SPAN: u64 = 100_000;

/// `SELECT "<folder>" (QRESYNC (<uidvalidity> <modseq>))` — the server answers
/// with the changed FETCHes and an explicit VANISHED list in one round trip.
pub fn select_command(folder: &str, uid_validity: u32, modseq: u64) -> String {
    let escaped = folder.replace('\\', "\\\\").replace('"', "\\\"");
    format!("SELECT \"{escaped}\" (QRESYNC ({uid_validity} {modseq}))")
}

/// Flatten VANISHED ranges into UIDs, refusing an implausible span rather than
/// trying to allocate it.
pub fn expand_ranges(ranges: &[std::ops::RangeInclusive<u32>]) -> Vec<u32> {
    let mut out = Vec::new();
    for r in ranges {
        let span = u64::from(*r.end()).saturating_sub(u64::from(*r.start())) + 1;
        if span > MAX_RANGE_SPAN {
            return Vec::new();
        }
        out.extend(r.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_qresync_select() {
        assert_eq!(
            select_command("INBOX", 1234, 98765),
            "SELECT \"INBOX\" (QRESYNC (1234 98765))"
        );
    }

    #[test]
    fn a_quote_in_the_folder_name_is_escaped() {
        assert_eq!(
            select_command("Odd\"Name", 1, 2),
            "SELECT \"Odd\\\"Name\" (QRESYNC (1 2))"
        );
    }

    #[test]
    fn ranges_expand_into_uids() {
        assert_eq!(expand_ranges(&[1..=3, 7..=7]), vec![1, 2, 3, 7]);
    }

    #[test]
    fn an_absurd_range_is_refused_rather_than_allocated() {
        // A malformed or hostile VANISHED must not try to build 4 billion u32s.
        assert!(expand_ranges(&[1..=u32::MAX]).is_empty());
    }
}
