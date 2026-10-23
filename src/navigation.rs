//! Explicit page navigation over a contiguous, user-loaded subset of a thread.
use std::collections::{BTreeMap, BTreeSet};

use nmbxd::{model::Post, tree::ThreadTree};

// Xdcmd's Thread.maxPage uses ceil(replyCount / 19), excluding the service Tips.
// https://github.com/TransparentLC/xdcmd/blob/master/xdnmb/model.py
const REPLIES_PER_PAGE: u64 = 19;

pub fn total_pages(reply_count: u64) -> u32 {
    reply_count
        .div_ceil(REPLIES_PER_PAGE)
        .clamp(1, u32::MAX as u64) as u32
}

pub fn parse_page(input: &str, total: u32) -> Option<u32> {
    input
        .trim()
        .parse::<u32>()
        .ok()
        .filter(|&page| page >= 1 && page <= total)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageMode {
    Navigate,
    Jump,
    Buffer,
    Refresh,
    /// Re-fetch an existing page to discover newly added replies.
    Recheck,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PageAction {
    Ignored,
    Cached,
    Fetch,
}

#[derive(Clone, Copy, Debug)]
struct Pending {
    page: u32,
    mode: PageMode,
}

#[derive(Default)]
pub struct Navigation {
    pages: BTreeMap<u32, Post>,
    current: u32,
    pending: Option<Pending>,
    total: u32,
}

impl Navigation {
    pub fn current(&self) -> u32 {
        self.current
    }

    pub fn total(&self) -> u32 {
        self.total.max(1)
    }

    pub fn loading(&self) -> bool {
        self.pending.is_some()
    }

    pub fn range(&self) -> Option<(u32, u32)> {
        Some((
            *self.pages.first_key_value()?.0,
            *self.pages.last_key_value()?.0,
        ))
    }

    pub fn before(&self) -> Option<u32> {
        self.range()?.0.checked_sub(1).filter(|&page| page > 0)
    }

    pub fn after(&self) -> Option<u32> {
        self.range()?
            .1
            .checked_add(1)
            .filter(|&page| page <= self.total())
    }

    pub fn request(&mut self, page: u32, mode: PageMode) -> PageAction {
        if self.loading() || page == 0 || page > self.total() {
            return PageAction::Ignored;
        }
        // Only jumps may introduce a gap in the subset, and they replace it.
        if let Some((first, last)) = self.range()
            && !matches!(mode, PageMode::Jump | PageMode::Refresh)
            && (page < first.saturating_sub(1) || page > last.saturating_add(1))
        {
            return PageAction::Ignored;
        }
        if !matches!(mode, PageMode::Refresh | PageMode::Recheck) && self.pages.contains_key(&page)
        {
            if mode == PageMode::Jump {
                self.pages.retain(|&number, _| number == page);
            }
            if mode != PageMode::Buffer {
                self.current = page;
            }
            return PageAction::Cached;
        }
        self.pending = Some(Pending { page, mode });
        PageAction::Fetch
    }

    /// Commit a matching successful request; failed requests never change data.
    pub fn receive(&mut self, page: u32, post: Option<Post>) -> bool {
        let Some(pending) = self.pending else {
            return false;
        };
        if pending.page != page {
            return false;
        }
        self.pending = None;
        let Some(post) = post else {
            return false;
        };
        if matches!(pending.mode, PageMode::Jump | PageMode::Refresh) {
            self.pages.clear();
        }
        self.total = total_pages(post.reply_count).max(page);
        self.pages.insert(page, post);
        if !matches!(pending.mode, PageMode::Buffer | PageMode::Recheck) || self.current == 0 {
            self.current = page;
        }
        true
    }

    pub fn tree(&self) -> Option<ThreadTree> {
        let mut root = self.pages.first_key_value()?.1.clone();
        root.replies.clear();
        let mut tree = ThreadTree::new(root);
        for (&page, post) in &self.pages {
            tree.merge_page(page, post.clone());
        }
        Some(tree)
    }

    /// Always put the root first, followed by this page's replies in ID order.
    pub fn linear_ids(&self) -> Vec<u64> {
        fn collect(post: &Post, ids: &mut BTreeSet<u64>) {
            if !post.is_tip() {
                ids.insert(post.id);
            }
            for reply in &post.replies {
                collect(reply, ids);
            }
        }
        let Some(post) = self.pages.get(&self.current) else {
            return vec![];
        };
        let mut ids = BTreeSet::new();
        collect(post, &mut ids);
        ids.remove(&post.id);
        std::iter::once(post.id).chain(ids).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(number: u32) -> Post {
        Post {
            id: 100,
            reply_count: 190,
            replies: vec![Post {
                id: 100 + number as u64,
                ..Post::default()
            }],
            ..Post::default()
        }
    }

    fn load(nav: &mut Navigation, number: u32, mode: PageMode) {
        assert_eq!(nav.request(number, mode), PageAction::Fetch);
        assert!(nav.receive(number, Some(page(number))));
    }

    #[test]
    fn bounds_follow_nineteen_real_replies_per_page() {
        for (count, expected) in [(0, 1), (1, 1), (19, 1), (20, 2), (38, 2), (39, 3)] {
            assert_eq!(total_pages(count), expected);
        }
        assert_eq!(total_pages(u64::MAX), u32::MAX);
        assert_eq!(parse_page(" 2 ", 3), Some(2));
        for invalid in ["0", "4", "-1", "1.5", "abc", "4294967296"] {
            assert_eq!(parse_page(invalid, 3), None);
        }
    }

    #[test]
    fn jumping_replaces_subset_only_after_success_and_buffers_are_explicit() {
        let mut nav = Navigation::default();
        load(&mut nav, 1, PageMode::Navigate);
        load(&mut nav, 2, PageMode::Navigate);
        assert_eq!(nav.request(8, PageMode::Jump), PageAction::Fetch);
        assert_eq!(nav.range(), Some((1, 2)));
        assert_eq!(nav.current(), 2);
        assert!(!nav.receive(8, None));
        assert_eq!(nav.range(), Some((1, 2)));
        load(&mut nav, 8, PageMode::Jump);
        assert_eq!(nav.range(), Some((8, 8)));
        assert_eq!((nav.before(), nav.after()), (Some(7), Some(9)));
        load(&mut nav, 7, PageMode::Buffer);
        load(&mut nav, 9, PageMode::Buffer);
        assert_eq!(nav.range(), Some((7, 9)));
        assert_eq!(nav.current(), 8);
        assert_eq!(nav.linear_ids(), vec![100, 108]);
        assert_eq!(nav.tree().unwrap().loaded_pages, BTreeSet::from([7, 8, 9]));
        assert_eq!(nav.request(7, PageMode::Navigate), PageAction::Cached);
        assert!(!nav.loading());
        assert_eq!(nav.current(), 7);
    }

    #[test]
    fn requests_are_gated_and_stale_or_duplicate_results_cannot_commit() {
        let mut nav = Navigation::default();
        assert_eq!(nav.request(1, PageMode::Navigate), PageAction::Fetch);
        assert_eq!(nav.request(1, PageMode::Navigate), PageAction::Ignored);
        assert!(!nav.receive(2, Some(page(2))));
        assert!(nav.loading());
        assert!(nav.receive(1, Some(page(1))));
        assert!(!nav.receive(1, Some(page(1))));
        assert_eq!(nav.request(5, PageMode::Buffer), PageAction::Ignored);
        assert_eq!(nav.request(11, PageMode::Jump), PageAction::Ignored);
        load(&mut nav, 2, PageMode::Buffer);
        assert_eq!(nav.request(2, PageMode::Jump), PageAction::Cached);
        assert_eq!(nav.range(), Some((2, 2)));
        assert_eq!(nav.current(), 2);
    }

    #[test]
    fn linear_rows_keep_root_first_and_remove_tips_and_overlaps() {
        let mut nav = Navigation::default();
        nav.request(1, PageMode::Navigate);
        let mut post = page(1);
        post.replies.extend([
            Post {
                id: 9_999_999,
                user_hash: "Tips".into(),
                ..Post::default()
            },
            Post {
                id: 50,
                ..Post::default()
            },
            Post {
                id: 101,
                ..Post::default()
            },
        ]);
        nav.receive(1, Some(post));
        assert_eq!(nav.linear_ids(), vec![100, 50, 101]);
        assert_eq!(nav.tree().unwrap().nodes.len(), 3);
    }

    #[test]
    fn refresh_replaces_subset_on_success_and_keeps_it_on_failure() {
        let mut nav = Navigation::default();
        load(&mut nav, 1, PageMode::Navigate);
        load(&mut nav, 8, PageMode::Jump);
        assert_eq!(nav.request(1, PageMode::Refresh), PageAction::Fetch);
        assert!(!nav.receive(1, None));
        assert_eq!(nav.range(), Some((8, 8)));
        assert_eq!(nav.request(1, PageMode::Refresh), PageAction::Fetch);
        assert!(nav.receive(1, Some(page(1))));
        assert_eq!(nav.current(), 1);
        assert_eq!(nav.range(), Some((1, 1)));
        assert_eq!(nav.linear_ids(), vec![100, 101]);
    }
}

#[cfg(test)]
mod recheck_tests {
    use super::*;

    #[test]
    fn recheck_discovers_new_pages_and_preserves_current_and_buffer() {
        let mut nav = Navigation::default();
        nav.request(1, PageMode::Navigate);
        nav.receive(
            1,
            Some(Post {
                id: 1,
                reply_count: 20,
                ..Post::default()
            }),
        );
        nav.request(2, PageMode::Buffer);
        nav.receive(
            2,
            Some(Post {
                id: 1,
                reply_count: 20,
                ..Post::default()
            }),
        );
        assert_eq!(nav.after(), None);
        assert_eq!(nav.request(2, PageMode::Recheck), PageAction::Fetch);
        assert!(!nav.receive(2, None));
        assert_eq!(nav.range(), Some((1, 2)));
        assert_eq!(nav.request(2, PageMode::Recheck), PageAction::Fetch);
        assert!(nav.receive(
            2,
            Some(Post {
                id: 1,
                reply_count: 39,
                ..Post::default()
            })
        ));
        assert_eq!(nav.range(), Some((1, 2)));
        assert_eq!(nav.current(), 1);
        assert_eq!(nav.after(), Some(3));
    }
}
