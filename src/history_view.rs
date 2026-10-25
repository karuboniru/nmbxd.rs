//! Local browsing and successful posting records, presented as a regular page.
use crate::*;
use chrono::TimeZone;
use rdnmb::history::{HistoryCategory, HistoryEntry};

const HISTORY_PAGE_SIZE: u32 = 40;

fn history_time(timestamp: i64) -> String {
    chrono::Local
        .timestamp_millis_opt(timestamp)
        .single()
        .map(|time| time.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "时间未知".into())
}

impl Island {
    pub(crate) fn open_history(&mut self, category: HistoryCategory, cx: &mut Context<Self>) {
        if !self.showing_history {
            self.save_reading_position();
        }
        self.showing_history = true;
        self.history_category = category;
        self.previews.clear();
        self.viewing_image = None;
        self.history_entries.clear();
        self.history_offset = 0;
        self.history_has_more = false;
        self.history_scroll = ScrollHandle::new();
        self.load_history(false, cx);
    }

    pub(crate) fn load_history(&mut self, append: bool, cx: &mut Context<Self>) {
        let Some(store) = &self.history else {
            self.notice = "本地历史记录不可用".into();
            cx.notify();
            return;
        };
        let offset = if append { self.history_offset } else { 0 };
        match store.list(self.history_category, HISTORY_PAGE_SIZE + 1, offset) {
            Ok(mut entries) => {
                self.history_has_more = entries.len() > HISTORY_PAGE_SIZE as usize;
                entries.truncate(HISTORY_PAGE_SIZE as usize);
                self.history_offset = offset.saturating_add(entries.len() as u32);
                if append {
                    self.history_entries.extend(entries);
                } else {
                    self.history_entries = entries;
                    self.history_scroll = ScrollHandle::new();
                }
                self.notice = "历史记录保存在本机 · 按最近时间排列".into();
            }
            Err(error) => {
                self.notice = format!("读取本地历史记录失败：{error}");
            }
        }
        cx.notify();
    }

    pub(crate) fn render_history(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let p = Palette::current(cx);
        let mut tabs = div().flex().flex_shrink_0().gap_2();
        for (id, label, category) in [
            ("history-browsing", "浏览历史", HistoryCategory::Browsing),
            ("history-threads", "发串历史", HistoryCategory::Threads),
            ("history-replies", "回复历史", HistoryCategory::Replies),
        ] {
            tabs = tabs.child(
                button(p, id, label)
                    .when(self.history_category == category, |d| {
                        d.bg(rgb(p.selected)).font_weight(FontWeight::SEMIBOLD)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_history(category, cx);
                    })),
            );
        }
        let mut entries = div()
            .id("history-scroll")
            .track_scroll(&self.history_scroll)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_4();
        for entry in &self.history_entries {
            let card = div()
                .w_full()
                .min_w_0()
                .p_4()
                .mb_3()
                .rounded_lg()
                .bg(rgb(p.surface))
                .border_1()
                .border_color(rgb(p.border));
            entries = entries.child(match entry {
                HistoryEntry::Browse(entry) => {
                    let id = entry.thread_id;
                    let forum = if entry.forum_name.is_empty() {
                        format!("板块 {}", entry.forum_id)
                    } else {
                        entry.forum_name.clone()
                    };
                    let title = if entry.title.trim().is_empty() {
                        format!("No.{id}")
                    } else {
                        format!("No.{id} · {}", entry.title)
                    };
                    let position = if entry.position.aggregate {
                        format!(
                            "聚合第 {}–{} 页 · 上次第 {} 页",
                            entry.position.first_page,
                            entry.position.last_page,
                            entry.position.page
                        )
                    } else {
                        format!("上次第 {} 页", entry.position.page)
                    };
                    card.id(("history-browse", id))
                        .cursor_pointer()
                        .hover(move |d| d.bg(rgb(p.hover)))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(p.muted))
                                .child(format!("{} · {forum}", history_time(entry.visited_at))),
                        )
                        .child(div().mt_2().font_weight(FontWeight::SEMIBOLD).child(title))
                        .when(!entry.preview.is_empty(), |d| {
                            d.child(div().mt_2().text_sm().child(entry.preview.clone()))
                        })
                        .child(
                            div()
                                .mt_3()
                                .text_xs()
                                .text_color(rgb(p.muted))
                                .child(format!("{position} · 已缓存 {} 页", entry.cached_pages)),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.open_thread(id, cx)))
                        .into_any_element()
                }
                HistoryEntry::Posted(entry) => {
                    let target = entry.target;
                    let thread_id = target.or(entry.thread_id);
                    let destination = target.map(|id| format!("回复 No.{id}")).unwrap_or_else(
                        || match thread_id {
                            Some(id) => format!("发串 No.{id} · 板块 {}", entry.forum_id),
                            None => format!("发串 · 板块 {}", entry.forum_id),
                        },
                    );
                    let mut card = card
                        .id(("history-posted", entry.id as u64))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(p.muted))
                                .child(format!("{} · {destination}", history_time(entry.sent_at))),
                        )
                        .when(!entry.title.is_empty(), |d| {
                            d.child(
                                div()
                                    .mt_2()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(entry.title.clone()),
                            )
                        })
                        .child(div().mt_2().text_sm().child(entry.body.clone()))
                        .when(entry.has_image, |d| {
                            d.child(
                                div()
                                    .mt_2()
                                    .text_xs()
                                    .text_color(rgb(p.muted))
                                    .child("含图片"),
                            )
                        })
                        .when(!entry.message.is_empty(), |d| {
                            d.child(
                                div()
                                    .mt_3()
                                    .text_xs()
                                    .text_color(rgb(p.muted))
                                    .child(format!("发送结果：{}", entry.message)),
                            )
                        });
                    if let Some(id) = thread_id {
                        card = card
                            .cursor_pointer()
                            .hover(move |d| d.bg(rgb(p.hover)))
                            .on_click(cx.listener(move |this, _, _, cx| this.open_thread(id, cx)));
                    } else {
                        card = card.child(
                            div()
                                .mt_3()
                                .text_xs()
                                .text_color(rgb(p.muted))
                                .child("未记录串号，暂无法直接打开"),
                        );
                    }
                    card.into_any_element()
                }
            });
        }
        if self.history_entries.is_empty() {
            let empty = if self.history.is_none() {
                "本地历史记录不可用，请查看下方提示。"
            } else {
                match self.history_category {
                    HistoryCategory::Browsing => "还没有浏览记录。打开串后会自动保存在本机。",
                    HistoryCategory::Threads => "还没有成功发送的发串记录。",
                    HistoryCategory::Replies => "还没有成功发送的回复记录。",
                }
            };
            entries = entries.child(div().p_5().text_sm().text_color(rgb(p.muted)).child(empty));
        }
        if self.history_has_more {
            entries = entries.child(
                button(p, "history-more", "加载更多")
                    .on_click(cx.listener(|this, _, _, cx| this.load_history(true, cx))),
            );
        }
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(heading(p, "本机记录", "历史记录".into()))
            .child(
                div()
                    .px_4()
                    .py_3()
                    .flex()
                    .id("history-toolbar")
                    .overflow_x_scroll()
                    .whitespace_nowrap()
                    .flex_shrink_0()
                    .min_h_0()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .child(tabs)
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .gap_2()
                            .child(button(p, "history-refresh", "刷新").on_click(
                                cx.listener(|this, _, _, cx| this.load_history(false, cx)),
                            ))
                            .child(button(p, "history-back", "返回阅读").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.showing_history = false;
                                    cx.notify();
                                },
                            ))),
                    ),
            )
            .child(entries)
            .into_any_element()
    }
}
