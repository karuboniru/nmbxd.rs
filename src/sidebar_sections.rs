//! Sidebar sections start with only timelines expanded.
use std::collections::HashSet;

pub(crate) struct SidebarSections {
    pub(crate) timelines: bool,
    expanded_forums: HashSet<i64>,
}

impl Default for SidebarSections {
    fn default() -> Self {
        Self {
            timelines: true,
            expanded_forums: HashSet::new(),
        }
    }
}

impl SidebarSections {
    pub(crate) fn forum_expanded(&self, id: i64) -> bool {
        self.expanded_forums.contains(&id)
    }

    pub(crate) fn toggle_forum(&mut self, id: i64) {
        if !self.expanded_forums.remove(&id) {
            self.expanded_forums.insert(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SidebarSections;

    #[test]
    fn only_timelines_start_expanded_and_sections_toggle_independently() {
        let mut sections = SidebarSections::default();
        assert!(sections.timelines);
        assert!(!sections.forum_expanded(1));
        assert!(!sections.forum_expanded(2));
        sections.toggle_forum(1);
        assert!(sections.forum_expanded(1));
        assert!(!sections.forum_expanded(2));
        sections.timelines = false;
        assert!(sections.forum_expanded(1));
        sections.toggle_forum(1);
        assert!(!sections.forum_expanded(1));
        assert!(!sections.timelines);
        // A newly loaded category starts collapsed regardless of existing toggles.
        assert!(!sections.forum_expanded(3));
    }
}
