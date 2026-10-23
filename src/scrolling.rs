//! A wheel gesture may request one page; layout changes alone never request one.
#[derive(Debug)]
pub struct ScrollPaging {
    pub failed: bool,
    pub check_pending: bool,
    armed: bool,
}

impl Default for ScrollPaging {
    fn default() -> Self {
        Self {
            failed: false,
            check_pending: false,
            armed: true,
        }
    }
}

impl ScrollPaging {
    /// Re-arm after the reader moves away from the bottom. A newly loaded short
    /// page stays disarmed, so wheel momentum cannot consume page after page.
    pub fn on_scroll(&mut self, at_bottom: bool, downward: bool) -> bool {
        if !at_bottom {
            self.armed = true;
            return false;
        }
        if !downward || self.failed || !self.armed {
            return false;
        }
        self.armed = false;
        true
    }

    pub fn retry(&mut self) {
        self.failed = false;
        self.armed = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_pages_cannot_chain_without_moving_away_or_manual_retry() {
        let mut paging = ScrollPaging::default();
        assert!(paging.on_scroll(true, true));
        assert!(!paging.on_scroll(true, true));
        assert!(!paging.on_scroll(false, false));
        assert!(paging.on_scroll(true, true));
        paging.retry();
        assert!(paging.on_scroll(true, true));
    }

    #[test]
    fn failure_requires_explicit_retry_even_after_scrolling_away() {
        let mut paging = ScrollPaging {
            failed: true,
            ..Default::default()
        };
        assert!(!paging.on_scroll(false, true));
        assert!(!paging.on_scroll(true, true));
        paging.retry();
        assert!(paging.on_scroll(true, true));
    }
}
