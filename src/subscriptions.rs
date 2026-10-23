//! Server-side subscription list. Unknown membership is never advertised as
//! confirmed absence: only loaded feed entries and successful mutations count.
use crate::*;

impl Island {
    pub(crate) fn load_subscriptions(&mut self, cx: &mut Context<Self>) {
        // A new list generation supersedes any in-flight forum/feed response.
        self.showing_history = false;
        self.showing_subscriptions = true;
        self.forum_epoch += 1;
        self.threads.clear();
        self.forum_page = 0;
        self.forum_end = false;
        self.forum_loading = false;
        self.forum_scroll = ScrollHandle::new();
        self.forum_paging = ScrollPaging::default();
        self.request_forum_next(cx);
    }

    pub(crate) fn change_subscription(&mut self, id: u64, add: bool, cx: &mut Context<Self>) {
        if !self.subscription_pending.insert(id) {
            return;
        }
        if self
            .tx
            .send(Request::FeedChange(
                self.settings.subscription_id.clone(),
                id,
                add,
            ))
            .is_err()
        {
            self.subscription_pending.remove(&id);
            self.notice = "后台服务不可用，请重启客户端".into();
        } else {
            self.notice = if add {
                "正在添加订阅…"
            } else {
                "正在移除订阅…"
            }
            .into();
        }
        cx.notify();
    }

    pub(crate) fn receive_subscription(
        &mut self,
        uuid: String,
        id: u64,
        add: bool,
        result: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        if uuid != self.settings.subscription_id {
            return;
        }
        self.subscription_pending.remove(&id);
        match result {
            Ok(_) => {
                if add {
                    self.subscribed.insert(id);
                } else {
                    self.subscribed.remove(&id);
                }
                self.notice = if add {
                    "已添加订阅"
                } else {
                    "已移除订阅"
                }
                .into();
                // Deleting shifts server pagination; restart rather than skip
                // the first item of the following page. Keep the reader intact.
                if self.showing_subscriptions {
                    self.load_subscriptions(cx);
                }
            }
            Err(error) => self.notice = format!("订阅操作未确认，请刷新列表核实：{error}"),
        }
        cx.notify();
    }

    pub(crate) fn subscription_button(&self, id: u64, cx: &mut Context<Self>) -> AnyElement {
        let known = self.subscribed.contains(&id);
        button(
            Palette::current(cx),
            ("toggle-subscription", id),
            if self.subscription_pending.contains(&id) {
                "处理中…"
            } else if known {
                "已订阅 · 移除"
            } else {
                "☆ 添加订阅"
            },
        )
        .on_click(cx.listener(move |this, _, _, cx| this.change_subscription(id, !known, cx)))
        .into_any_element()
    }
}
