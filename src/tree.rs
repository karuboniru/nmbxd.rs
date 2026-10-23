//! Deterministic reference trees built from the pages already in memory.

use std::collections::{BTreeMap, BTreeSet};

use crate::model::{Post, references};

#[derive(Clone, Debug, PartialEq)]
pub struct TreeNode {
    pub post: Post,
    pub parent: Option<u64>,
    pub children: Vec<u64>,
    pub references: Vec<u64>,
    pub missing_references: Vec<u64>,
}

#[derive(Clone, Debug)]
pub struct ThreadTree {
    pub root_id: u64,
    pub nodes: BTreeMap<u64, TreeNode>,
    pub loaded_pages: BTreeSet<u32>,
    initial: Post,
    pages: BTreeMap<u32, Post>,
}

impl ThreadTree {
    /// Start with a thread and any replies it already contains. Page numbers are
    /// recorded explicitly by `merge_page`, rather than inferred here.
    pub fn new(root: Post) -> Self {
        let mut tree = Self {
            root_id: root.id,
            nodes: BTreeMap::new(),
            loaded_pages: BTreeSet::new(),
            initial: root,
            pages: BTreeMap::new(),
        };
        tree.rebuild();
        tree
    }

    /// Replace a page snapshot and rebuild using every currently loaded page.
    /// Lower numbered pages win when their post IDs overlap; the initial
    /// snapshot supplies any posts absent from the explicit page snapshots.
    pub fn merge_page(&mut self, page: u32, thread: Post) {
        self.pages.insert(page, thread);
        self.loaded_pages.insert(page);
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let mut posts = BTreeMap::new();
        for thread in self.pages.values() {
            collect_posts(thread, &mut posts);
        }
        collect_posts(&self.initial, &mut posts);

        self.nodes = posts
            .into_iter()
            .map(|(id, post)| {
                let references = references(&post.content);
                (
                    id,
                    TreeNode {
                        post,
                        parent: None,
                        children: Vec::new(),
                        references,
                        missing_references: Vec::new(),
                    },
                )
            })
            .collect();

        let known: BTreeSet<_> = self.nodes.keys().copied().collect();
        let mut parents = BTreeMap::new();
        for (&id, node) in &mut self.nodes {
            node.missing_references = node
                .references
                .iter()
                .copied()
                .filter(|target| !known.contains(target))
                .collect();
            if id == self.root_id {
                continue;
            }
            let parent = node
                .references
                .iter()
                .copied()
                .find(|&target| {
                    target != id && known.contains(&target) && !would_cycle(id, target, &parents)
                })
                .unwrap_or(self.root_id);
            node.parent = Some(parent);
            parents.insert(id, parent);
        }

        // Both node iteration and child insertion use ascending post IDs.
        for (id, parent) in parents {
            if let Some(node) = self.nodes.get_mut(&parent) {
                node.children.push(id);
            }
        }
    }
}

fn collect_posts(post: &Post, posts: &mut BTreeMap<u64, Post>) {
    // The service appends this informational record to page responses; it is
    // not a forum post and must not become a reference-tree node.
    if !post.is_tip() {
        posts.entry(post.id).or_insert_with(|| {
            let mut node_post = post.clone();
            node_post.replies.clear();
            node_post
        });
    }
    for reply in &post.replies {
        collect_posts(reply, posts);
    }
}

fn would_cycle(id: u64, target: u64, parents: &BTreeMap<u64, u64>) -> bool {
    let mut current = target;
    loop {
        if current == id {
            return true;
        }
        match parents.get(&current) {
            Some(&parent) => current = parent,
            None => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn post(id: u64, content: &str) -> Post {
        Post {
            id,
            content: content.into(),
            ..Post::default()
        }
    }

    fn page(replies: Vec<Post>) -> Post {
        Post {
            replies,
            ..post(1, "thread")
        }
    }

    #[test]
    fn unresolved_reference_moves_when_its_post_arrives() {
        let mut tree = ThreadTree::new(post(1, "thread"));
        tree.merge_page(1, page(vec![post(20, ">>No.10")]));
        assert_eq!(tree.nodes[&20].parent, Some(1));
        assert_eq!(tree.nodes[&20].missing_references, vec![10]);

        tree.merge_page(2, page(vec![post(10, "target")]));
        assert_eq!(tree.nodes[&20].parent, Some(10));
        assert!(tree.nodes[&20].missing_references.is_empty());
        assert_eq!(tree.nodes[&1].children, vec![10]);
        assert_eq!(tree.nodes[&10].children, vec![20]);
        assert_eq!(tree.loaded_pages, BTreeSet::from([1, 2]));
    }

    #[test]
    fn preserves_all_references_and_chooses_first_known_target() {
        let tree = ThreadTree::new(page(vec![
            post(10, "target"),
            post(20, ">>No.99 >>No.20 >>No.10 >>No.1 >>No.88"),
        ]));
        assert_eq!(tree.nodes[&20].references, vec![99, 20, 10, 1, 88]);
        assert_eq!(tree.nodes[&20].missing_references, vec![99, 88]);
        assert_eq!(tree.nodes[&20].parent, Some(10));
        assert_eq!(tree.nodes[&1].parent, None);
    }

    #[test]
    fn forward_references_work_and_cycles_try_the_next_reference() {
        let tree = ThreadTree::new(page(vec![
            post(10, ">>No.20"),
            post(20, ">>No.10 >>No.30"),
            post(30, ">>No.20"),
        ]));
        assert_eq!(tree.nodes[&10].parent, Some(20));
        assert_eq!(tree.nodes[&20].parent, Some(30));
        assert_eq!(tree.nodes[&30].parent, Some(1));
        assert_eq!(tree.nodes[&1].children, vec![30]);
    }

    #[test]
    fn overlapping_pages_are_deduplicated_independent_of_load_order() {
        let first = page(vec![post(20, "first page version"), post(10, "early")]);
        let second = page(vec![post(30, ">>No.20"), post(20, "second page version")]);
        let mut ascending = ThreadTree::new(post(1, "thread"));
        ascending.merge_page(1, first.clone());
        ascending.merge_page(2, second.clone());
        let mut descending = ThreadTree::new(post(1, "thread"));
        descending.merge_page(2, second);
        descending.merge_page(1, first);

        assert_eq!(ascending.nodes, descending.nodes);
        assert_eq!(ascending.nodes.len(), 4);
        assert_eq!(ascending.nodes[&20].post.content, "first page version");
        assert_eq!(ascending.nodes[&1].children, vec![10, 20]);
        assert_eq!(ascending.nodes[&30].parent, Some(20));
        assert!(
            ascending
                .nodes
                .values()
                .all(|node| node.post.replies.is_empty())
        );
    }

    #[test]
    fn reloading_a_page_replaces_its_snapshot() {
        let mut tree = ThreadTree::new(post(1, "thread"));
        tree.merge_page(1, page(vec![post(10, "old")]));
        tree.merge_page(1, page(vec![post(20, "new")]));
        assert!(!tree.nodes.contains_key(&10));
        assert_eq!(tree.nodes[&1].children, vec![20]);
        assert_eq!(tree.loaded_pages, BTreeSet::from([1]));
    }

    #[test]
    fn tips_are_filtered_but_ordinary_posts_with_similar_fields_remain() {
        let mut tip = post(9_999_999, "service record");
        tip.user_hash = "Tips".into();
        let mut ordinary = post(10, "ordinary");
        ordinary.user_hash = "Tips".into();
        let tree = ThreadTree::new(page(vec![tip, ordinary]));
        assert_eq!(tree.nodes.len(), 2);
        assert!(tree.nodes.contains_key(&10));
    }
}
