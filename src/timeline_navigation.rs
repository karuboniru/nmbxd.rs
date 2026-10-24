//! Timelines have a separate ID namespace and are never posting destinations.
use crate::*;

pub(crate) fn posting_forum(timeline: Option<&Timeline>, forum: i64) -> i64 {
    if timeline.is_some() { 4 } else { forum }
}

fn limit_reached(timeline: Option<&Timeline>, page: u32) -> bool {
    timeline.is_some_and(|timeline| timeline.max_page > 0 && page >= timeline.max_page)
}

impl Island {
    pub(crate) fn timeline_limit_reached(&self, page: u32) -> bool {
        limit_reached(self.timeline.as_ref(), page)
    }

    fn request_timelines(&mut self, cx: &mut Context<Self>) {
        if self.timelines_loading {
            return;
        }
        self.timelines_loading = true;
        self.timelines_error = None;
        if self.tx.send(Request::Timelines).is_err() {
            self.timelines_loading = false;
            self.timelines_error = Some("后台服务不可用，请重启客户端".into());
        }
        cx.notify();
    }

    pub(crate) fn load_timeline(&mut self, timeline: Timeline, cx: &mut Context<Self>) {
        self.showing_history = false;
        self.showing_subscriptions = false;
        self.timeline = Some(timeline);
        self.reader_target_changed();
        // Switching sources invalidates responses even when their IDs overlap.
        self.forum_epoch += 1;
        self.forum_loading = false;
        self.threads.clear();
        self.forum_page = 0;
        self.forum_end = false;
        self.forum_scroll = ScrollHandle::new();
        self.forum_paging = ScrollPaging::default();
        self.request_forum_next(cx);
    }

    pub(crate) fn timeline_sidebar(
        &self,
        mut sidebar: Stateful<Div>,
        p: Palette,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        sidebar = sidebar.child(
            button(
                p,
                "timeline-group",
                if self.sidebar_sections.timelines {
                    "▾ 时间线"
                } else {
                    "▸ 时间线"
                },
            )
            .w_full()
            .mt_3()
            .mb_2()
            .text_color(rgb(p.muted))
            .on_click(cx.listener(|this, _, _, cx| {
                this.sidebar_sections.timelines = !this.sidebar_sections.timelines;
                cx.notify();
            })),
        );
        if !self.sidebar_sections.timelines {
            return sidebar;
        }
        for timeline in &self.timelines {
            let timeline = timeline.clone();
            let selected = !self.showing_history
                && !self.showing_subscriptions
                && self
                    .timeline
                    .as_ref()
                    .is_some_and(|item| item.id == timeline.id);
            sidebar = sidebar.child(
                button(p, ("timeline", timeline.id as u64), timeline.display_name())
                    .w_full()
                    .mb_1()
                    .when(selected, |d| {
                        d.bg(rgb(p.selected)).font_weight(FontWeight::SEMIBOLD)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.load_timeline(timeline.clone(), cx);
                    })),
            );
        }
        if self.timelines_loading {
            sidebar = sidebar.child(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(rgb(p.muted))
                    .child("正在获取时间线…"),
            );
        } else if let Some(error) = &self.timelines_error {
            sidebar = sidebar
                .child(
                    div()
                        .px_3()
                        .py_2()
                        .text_xs()
                        .text_color(rgb(p.muted))
                        .child(format!("时间线获取失败：{error}")),
                )
                .child(
                    button(p, "retry-timelines", "重新获取时间线")
                        .on_click(cx.listener(|this, _, _, cx| this.request_timelines(cx))),
                );
        } else if self.timelines.is_empty() {
            sidebar = sidebar.child(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(rgb(p.muted))
                    .child("暂无时间线"),
            );
        }
        sidebar
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    #[test]
    fn timeline_ids_never_become_posting_forums() {
        let timeline = Timeline {
            id: 5,
            ..Default::default()
        };
        assert_eq!(posting_forum(Some(&timeline), 17), 4);
        assert_eq!(posting_forum(None, 17), 17);
        assert_eq!(
            drafts::Target::new(None, posting_forum(Some(&timeline), 17)),
            drafts::Target::Forum(4)
        );
    }

    #[test]
    fn known_timeline_limits_cap_requests_and_zero_is_unknown() {
        let timeline = Timeline {
            id: 4,
            max_page: 2,
            ..Default::default()
        };
        assert!(!limit_reached(Some(&timeline), 1));
        assert!(limit_reached(Some(&timeline), 2));
        assert!(limit_reached(Some(&timeline), 3));
        assert!(!limit_reached(Some(&Timeline::default()), u32::MAX));
        assert!(!limit_reached(None, u32::MAX));
    }
}
