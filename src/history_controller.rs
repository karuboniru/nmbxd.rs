//! Persistence and resume hooks shared by every entry point into a thread.
use crate::*;

impl Island {
    pub(crate) fn capture_reading_position(&self) -> Option<ReadingPosition> {
        if self.selected.is_none() || self.navigation.range().is_none() {
            return None;
        }
        let (first_page, last_page) = self.navigation.range()?;
        let (index, delta) = self.reading_scroll.logical_scroll_top();
        let anchor = self.rows().get(index).map(|(id, _)| *id);
        let page = anchor
            .and_then(|id| self.navigation.page_for_post(id))
            .unwrap_or(self.navigation.current().max(1));
        Some(ReadingPosition {
            page,
            offset: self.reading_scroll.offset().y.into(),
            anchor,
            anchor_offset: delta.into(),
            aggregate: self.aggregate,
            first_page,
            last_page,
        })
    }

    pub(crate) fn save_reading_position(&mut self) {
        // Never replace a saved location with the temporary top of a loading view.
        if self.reading_restore.is_some() || self.showing_history {
            return;
        }
        if let (Some(store), Some(id), Some(position)) = (
            &self.history,
            self.selected,
            self.capture_reading_position(),
        ) && let Err(error) = store.save_position(id, &position)
        {
            self.notice = format!("保存阅读位置失败：{error}");
        }
    }

    pub(crate) fn schedule_position_save(&mut self, cx: &mut Context<Self>) {
        self.position_save_revision = self.position_save_revision.wrapping_add(1);
        let revision = self.position_save_revision;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(400))
                .await;
            let _ = this.update(cx, |this, _| {
                if this.position_save_revision == revision {
                    this.save_reading_position();
                }
            });
        })
        .detach();
    }

    fn record_current_visit(&mut self) {
        let Some(id) = self.selected else {
            return;
        };
        let Some(store) = &self.history else {
            return;
        };
        let post = self
            .navigation
            .pages()
            .next()
            .map(|(_, post)| post)
            .or_else(|| self.threads.iter().find(|post| post.id == id));
        let previous = store.last_visit(id).ok().flatten();
        let title = post
            .map(|post| post.title.clone())
            .or_else(|| previous.as_ref().map(|entry| entry.title.clone()))
            .unwrap_or_default();
        let preview = post
            .map(|post| post.text().chars().take(280).collect::<String>())
            .or_else(|| previous.as_ref().map(|entry| entry.preview.clone()))
            .unwrap_or_default();
        let fid = post
            .map(|post| post.fid)
            .filter(|id| *id != 0)
            .or_else(|| previous.as_ref().map(|entry| entry.forum_id))
            .unwrap_or(self.forum);
        let forum_name = self
            .groups
            .iter()
            .flat_map(|group| &group.forums)
            .find(|forum| forum.id == fid)
            .map(|forum| forum.display_name())
            .or_else(|| previous.as_ref().map(|entry| entry.forum_name.clone()))
            .unwrap_or_default();
        if let Err(error) = store.record_visit(id, &title, &preview, fid, &forum_name) {
            self.notice = format!("保存浏览历史失败：{error}");
        }
    }

    pub(crate) fn open_saved_thread(&mut self, id: u64, cx: &mut Context<Self>) {
        self.save_reading_position();
        self.showing_history = false;
        if self.selected == Some(id) {
            self.record_current_visit();
            cx.notify();
            return;
        }
        let saved = self
            .history
            .as_ref()
            .and_then(|store| store.last_visit(id).ok().flatten());
        self.thread_end_checked = false;
        self.cached_short_check = false;
        self.cached_check_scheduled = false;
        self.selected = Some(id);
        self.reader_target_changed();
        self.tree = None;
        self.navigation = Navigation::default();
        self.reading_restore = saved.as_ref().map(|entry| entry.position.clone());
        self.restore_scheduled = false;
        self.aggregate = saved.as_ref().is_some_and(|entry| entry.position.aggregate);
        self.reset_reading_scroll();
        self.reading_paging = ScrollPaging::default();
        self.reading_reset_pending = false;
        self.reading_retry = None;
        self.thread_epoch = self.thread_epoch.wrapping_add(1);
        self.collapsed.clear();
        let page = saved.as_ref().map_or(1, |entry| entry.position.page.max(1));
        let mut cached = Vec::new();
        if let Some(store) = &self.history
            && let Ok(Some(post)) = store.cached_page(id, page)
        {
            cached.push((page, post));
            if let Some(entry) = &saved {
                // Restore a bounded contiguous window around the anchor. Other
                // visited pages remain on disk and load on demand at either edge.
                let first = entry
                    .position
                    .first_page
                    .max(page.saturating_sub(50))
                    .max(1);
                let last = entry.position.last_page.min(page.saturating_add(50));
                for number in (first..page).rev() {
                    match store.cached_page(id, number) {
                        Ok(Some(post)) => cached.push((number, post)),
                        _ => break,
                    }
                }
                for number in page.saturating_add(1)..=last {
                    match store.cached_page(id, number) {
                        Ok(Some(post)) => cached.push((number, post)),
                        _ => break,
                    }
                }
            }
        }
        cached.sort_by_key(|(page, _)| *page);
        if self.navigation.restore(cached, page) {
            self.cached_short_check = true;
            self.tree = self.navigation.tree();
            self.notice = format!("已从本地缓存恢复第 {page} 页 · 可清除本串缓存后重载");
            self.record_current_visit();
            cx.notify();
        } else {
            self.navigation.allow_resume_page(page);
            self.record_current_visit();
            self.request_page(page, PageMode::Navigate, cx);
        }
    }

    pub(crate) fn receive_thread_page(
        &mut self,
        page: u32,
        result: Result<Post, String>,
        cached: bool,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(post) => {
                let anchor = if self.reading_restore.is_none() && !self.reading_reset_pending {
                    self.capture_reading_position()
                } else {
                    None
                };
                let snapshot = post.clone();
                if self.navigation.receive(page, Some(post)) {
                    self.cached_short_check |= cached;
                    self.tree = self.navigation.tree();
                    if self.reading_reset_pending {
                        self.reading_restore = None;
                        self.reset_reading_scroll();
                    } else if self.reading_restore.is_none() {
                        self.reading_restore = anchor;
                    }
                    self.reading_reset_pending = false;
                    self.reading_retry = None;
                    self.reading_paging.failed = false;
                    self.notice = if cached {
                        format!("第 {page} 页 · 使用本地缓存")
                    } else {
                        format!("已加载第 {page} 页 · 新内容已接入当前串")
                    };
                    self.record_current_visit();
                    if !cached
                        && let Some(store) = &self.history
                        && let Err(error) = store.cache_page(snapshot.id, page, &snapshot)
                    {
                        self.notice = format!("页面已加载，但缓存保存失败：{error}");
                    }
                    self.save_reading_position();
                }
            }
            Err(error) => {
                self.navigation.receive(page, None);
                self.reading_reset_pending = false;
                self.reading_paging.failed = true;
                self.notice = if self.tree.is_some() {
                    format!("第 {page} 页加载失败，已保留当前内容：{error}")
                } else {
                    format!("第 {page} 页加载失败：{error}")
                };
            }
        }
        cx.notify();
    }

    pub(crate) fn schedule_reading_restore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.reading_restore.is_none()
            || self.restore_scheduled
            || self.showing_history
            || self.navigation.loading()
            || self.tree.is_none()
        {
            return;
        }
        self.restore_scheduled = true;
        let epoch = self.thread_epoch;
        let revision = self.reading_revision;
        cx.on_next_frame(window, move |this, _, cx| {
            this.restore_scheduled = false;
            if this.thread_epoch != epoch
                || this.reading_revision != revision
                || this.showing_history
            {
                return;
            }
            let Some(position) = this.reading_restore.take() else {
                return;
            };
            let y = position
                .anchor
                .and_then(|id| this.rows().iter().position(|(row, _)| *row == id))
                .and_then(|index| this.reading_scroll.bounds_for_item(index))
                .map(|bounds| {
                    this.reading_scroll.bounds().top() - bounds.top() + px(position.anchor_offset)
                })
                .unwrap_or(px(position.offset));
            let y = y.min(px(0.)).max(-this.reading_scroll.max_offset().height);
            this.reading_scroll.set_offset(point(px(0.), y));
            this.save_reading_position();
            cx.notify();
        });
    }

    /// Recheck the loaded trailing page once during this visit to the thread.
    /// Mark before sending: a failure or a short response must not start a loop.
    pub(crate) fn check_thread_end_once(&mut self, cx: &mut Context<Self>) {
        if self.navigation.loading() || self.reading_paging.failed || self.showing_history {
            return;
        }
        let (Some(_), Some((_, last))) = (self.selected, self.navigation.range()) else {
            return;
        };
        if !self.thread_end_checked {
            self.thread_end_checked = true;
            self.request_page(last, PageMode::Recheck, cx);
        }
    }

    pub(crate) fn schedule_cached_short_check(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.cached_short_check
            || self.cached_check_scheduled
            || self.showing_history
            || self.navigation.loading()
            || self.reading_restore.is_some()
            || self.tree.is_none()
        {
            return;
        }
        self.cached_check_scheduled = true;
        let epoch = self.thread_epoch;
        let revision = self.reading_revision;
        // Scroll bounds are valid only after the cached content has been laid out.
        cx.on_next_frame(window, move |this, _, cx| {
            if this.thread_epoch != epoch {
                return;
            }
            this.cached_check_scheduled = false;
            if this.reading_revision != revision
                || this.showing_history
                || this.navigation.loading()
                || this.reading_restore.is_some()
            {
                cx.notify();
                return;
            }
            if this.reading_scroll.bounds().size.height <= px(0.) {
                return;
            }
            this.cached_short_check = false;
            if this.reading_scroll.max_offset().height <= px(2.) {
                this.check_thread_end_once(cx);
            }
        });
    }

    pub(crate) fn clear_thread_cache(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        self.save_reading_position();
        if let Some(store) = &self.history {
            match store.clear_thread_cache(id) {
                Ok(urls) => self
                    .media
                    .update(cx, |media, cx| media.invalidate(&urls, cx)),
                Err(error) => {
                    self.notice = format!("清除缓存失败：{error}");
                    cx.notify();
                    return;
                }
            }
        }
        self.cached_short_check = false;
        self.cached_check_scheduled = false;
        // Invalidates in-flight old responses before they can repopulate cache.
        self.thread_epoch = self.thread_epoch.wrapping_add(1);
        self.navigation = Navigation::default();
        self.tree = None;
        self.reading_restore = None;
        self.restore_scheduled = false;
        self.collapsed.clear();
        self.reset_reading_scroll();
        self.reading_paging = ScrollPaging::default();
        self.request_page(1, PageMode::Refresh, cx);
    }
}
