use std::collections::HashMap;

/// Where a message lives. Never shown to the model; refs stand in for it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MessageLoc {
    pub folder: String,
    pub uid: u32,
}

/// Short refs (`m1`, `m2`, …) minted per request. The model can only name a
/// message it was shown, because anything else fails to resolve.
#[derive(Debug, Default)]
pub struct RefTable {
    by_ref: HashMap<String, MessageLoc>,
    by_loc: HashMap<MessageLoc, String>,
}

impl RefTable {
    pub fn mint(&mut self, loc: MessageLoc) -> String {
        if let Some(r) = self.by_loc.get(&loc) {
            return r.clone();
        }
        let r = format!("m{}", self.by_ref.len() + 1);
        self.by_ref.insert(r.clone(), loc.clone());
        self.by_loc.insert(loc, r.clone());
        r
    }

    pub fn resolve(&self, r: &str) -> Option<&MessageLoc> {
        self.by_ref.get(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loc(folder: &str, uid: u32) -> MessageLoc {
        MessageLoc { folder: folder.to_string(), uid }
    }

    #[test]
    fn mints_sequential_refs_and_reuses_them() {
        let mut t = RefTable::default();
        assert_eq!(t.mint(loc("INBOX", 7)), "m1");
        assert_eq!(t.mint(loc("Junk", 7)), "m2");
        assert_eq!(t.mint(loc("INBOX", 7)), "m1");
        assert_eq!(t.resolve("m2"), Some(&loc("Junk", 7)));
    }

    #[test]
    fn unknown_refs_do_not_resolve() {
        let mut t = RefTable::default();
        t.mint(loc("INBOX", 1));
        assert_eq!(t.resolve("m9"), None);
        assert_eq!(t.resolve("INBOX/1"), None);
    }
}
