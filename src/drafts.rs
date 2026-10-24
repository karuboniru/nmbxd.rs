//! Draft ownership is independent of browsing; never carry content across targets.
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Reply(u64),
    Forum(i64),
}

impl Target {
    pub fn new(thread: Option<u64>, forum: i64) -> Self {
        thread.map(Self::Reply).unwrap_or(Self::Forum(forum))
    }

    /// Forum drafts belong to an explicitly selected, real posting forum.
    /// Reading a different forum or timeline does not change that destination.
    pub fn can_send(self, selected: Option<u64>, forums: impl IntoIterator<Item = i64>) -> bool {
        match self {
            Self::Reply(id) => selected == Some(id),
            Self::Forum(id) => id > 0 && forums.into_iter().any(|forum| forum == id),
        }
    }

    pub fn select_forum(
        self,
        forum: i64,
        sending: bool,
        forums: impl IntoIterator<Item = i64>,
    ) -> Option<Self> {
        if sending || matches!(self, Self::Reply(_)) {
            return None;
        }
        let target = Self::Forum(forum);
        target.can_send(None, forums).then_some(target)
    }
}

pub struct Drafts<T>(HashMap<Target, T>);

impl<T> Default for Drafts<T> {
    fn default() -> Self {
        Self(HashMap::new())
    }
}

impl<T> Drafts<T> {
    pub fn switch(&mut self, from: Target, to: Target, current: T, empty: impl FnOnce() -> T) -> T {
        if from == to {
            return current;
        }
        self.0.insert(from, current);
        self.0.remove(&to).unwrap_or_else(empty)
    }
}

/// Append a reference on its own line, without creating a leading blank line.
pub fn quote_suffix(current: &str, id: u64) -> String {
    let separator = if current.is_empty() || current.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    format!("{separator}>>No.{id}\n")
}

#[cfg(test)]
mod tests {
    use super::{Drafts, Target, quote_suffix};

    #[test]
    fn references_start_on_the_first_line_and_leave_a_line_for_typing() {
        assert_eq!(quote_suffix("", 123), ">>No.123\n");
        assert_eq!(quote_suffix("中文😀", 123), "\n>>No.123\n");
        assert_eq!(quote_suffix("existing\n", 123), ">>No.123\n");
        assert_eq!(quote_suffix("existing\r\n", 123), ">>No.123\n");
        assert_eq!(quote_suffix("intentional\n\n", 123), ">>No.123\n");
    }

    #[test]
    fn reply_a_never_inherits_reply_b_and_b_can_be_restored() {
        let a = Target::Reply(10);
        let b = Target::Reply(20);
        let mut drafts = Drafts::default();
        let a_draft = drafts.switch(b, a, ("B body", "B title", Some("B image")), || {
            ("", "", None)
        });
        assert_eq!(a_draft, ("", "", None));
        let b_draft = drafts.switch(
            a,
            b,
            ("A body", "A title", Some("A image")),
            || unreachable!(),
        );
        assert_eq!(b_draft, ("B body", "B title", Some("B image")));
        // Clearing/sending B cannot clear A's parked draft.
        let restored = drafts.switch(b, a, ("", "", None), || unreachable!());
        assert_eq!(restored, ("A body", "A title", Some("A image")));
    }

    #[test]
    fn forum_drafts_are_separate_and_same_target_keeps_current_state() {
        let mut drafts = Drafts::default();
        let a = Target::Forum(4);
        let b = Target::Forum(5);
        assert_eq!(
            drafts.switch(a, a, "active", || panic!("same target reset")),
            "active"
        );
        assert_eq!(drafts.switch(a, b, "forum4", || ""), "");
        assert_eq!(
            drafts.switch(b, Target::Reply(4), "forum5", || "reply"),
            "reply"
        );
        assert_eq!(
            drafts.switch(Target::Reply(4), a, "reply4", || unreachable!()),
            "forum4"
        );
    }

    #[test]
    fn send_guard_validates_reply_or_explicit_posting_destination() {
        assert!(Target::Reply(10).can_send(Some(10), [4, 5]));
        assert!(!Target::Reply(20).can_send(Some(10), [4, 5]));
        assert!(!Target::Reply(20).can_send(None, [4, 5]));
        assert!(Target::Forum(4).can_send(Some(10), [4, 5]));
        assert!(!Target::Forum(4).can_send(Some(10), [5]));
        assert!(Target::Forum(5).can_send(None, [4, 5]));
        assert!(!Target::Forum(0).can_send(None, [0, 4]));
        assert!(!Target::Forum(-1).can_send(None, [-1, 4]));
        assert_eq!(Target::new(Some(10), 999), Target::Reply(10));
    }

    #[test]
    fn destination_switch_preserves_all_draft_fields_and_rejects_replies_or_sends() {
        let from = Target::Forum(4);
        let to = from.select_forum(5, false, [4, 5]).unwrap();
        assert_eq!(to, Target::Forum(5));
        assert_eq!(from.select_forum(7, false, [4, 5]), None);
        assert_eq!(from.select_forum(5, true, [4, 5]), None);
        assert_eq!(Target::Reply(4).select_forum(5, false, [4, 5]), None);
        let mut drafts = Drafts::default();
        assert_eq!(
            drafts.switch(from, to, ("body", "title", "image", "cookie"), || (
                "", "", "", ""
            )),
            ("", "", "", "")
        );
        assert_eq!(
            drafts.switch(
                to,
                from,
                ("other", "", "", "other cookie"),
                || unreachable!()
            ),
            ("body", "title", "image", "cookie")
        );
    }
}
