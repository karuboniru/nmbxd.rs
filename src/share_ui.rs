//! Sharing and the explicit link / number navigation entry point.
use crate::*;
use share_links::OpenTarget;

impl Island {
    pub(crate) fn copy_number(&mut self, id: u64, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(id.to_string()));
        self.notice = format!("已复制串号 No.{id}");
        cx.notify();
    }

    pub(crate) fn copy_link(&mut self, url: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
        self.notice = if url.starts_with("rdnmb://") {
            "消息链接已复制；对方可粘贴到 R岛左上角“打开…”查看".into()
        } else {
            "链接已复制，可分享给他人".into()
        };
        cx.notify();
    }

    pub(crate) fn number_button(
        &self,
        element_id: impl Into<ElementId>,
        id: u64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = Palette::current(cx);
        div()
            .id(element_id)
            .cursor_pointer()
            .hover(move |d| d.text_color(rgb(p.foreground)).underline())
            .child(format!("No.{id}"))
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.copy_number(id, cx);
            }))
            .into_any_element()
    }

    pub(crate) fn copy_link_button(
        &self,
        element_id: impl Into<ElementId>,
        id: u64,
        thread: Option<u64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let url = match thread {
            Some(root) if root != id => {
                let thread_url = share_links::thread_url(&self.settings.website_base, root);
                match (self.selected == Some(root))
                    .then(|| self.navigation.page_for_post(id))
                    .flatten()
                {
                    Some(page) => format!("{thread_url}?page={page}&r={id}"),
                    None => format!("{thread_url}?r={id}"),
                }
            }
            Some(root) => share_links::thread_url(&self.settings.website_base, root),
            None => share_links::message_url(&self.settings.website_base, id),
        };
        let label = if thread.is_some() {
            "复制链接"
        } else {
            "复制消息链接"
        };
        button(Palette::current(cx), element_id, label)
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.copy_link(url.clone(), cx);
            }))
            .into_any_element()
    }

    // None infers the destination from a link; the two explicit modes accept IDs only.
    pub(crate) fn open_shared(&mut self, as_thread: Option<bool>, cx: &mut Context<Self>) {
        let value = self.open_input.read(cx).value().to_string();
        let parsed = share_links::parse_target(&value, &self.settings.website_base);
        let target = match parsed {
            Ok(target) => target,
            Err(error) => {
                self.open_error = Some(error);
                cx.notify();
                return;
            }
        };
        let target = if let Some(as_thread) = as_thread {
            if value.contains('/') || value.contains(':') {
                self.open_error = Some("请输入串号或消息 ID；链接请使用“打开链接”".into());
                cx.notify();
                return;
            }
            let id = match target {
                OpenTarget::Thread(id) | OpenTarget::Message(id) => id,
            };
            if as_thread {
                OpenTarget::Thread(id)
            } else {
                OpenTarget::Message(id)
            }
        } else {
            target
        };
        if self.settings_panel.is_some() {
            self.open_error = Some("请先保存或关闭设置，再打开链接或串号".into());
            cx.notify();
            return;
        }
        self.open_error = None;
        self.open_link_panel = false;
        self.viewing_image = None;
        self.previews.clear();
        match target {
            OpenTarget::Thread(id) => self.open_thread(id, cx),
            OpenTarget::Message(id) => {
                self.open_preview(id, cx);
                self.set_preview_thread_hint(share_links::enclosing_thread(
                    &value,
                    &self.settings.website_base,
                ));
            }
        }
        cx.notify();
    }

    pub(crate) fn share_open_panel(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = Palette::current(cx);
        div()
            .p_3()
            .flex_shrink_0()
            .bg(rgb(p.surface))
            .border_b_1()
            .border_color(rgb(p.border))
            .child(Input::new(&self.open_input))
            .child(
                div()
                    .mt_2()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(
                        button(p, "open-shared-link", "打开链接")
                            .on_click(cx.listener(|this, _, _, cx| this.open_shared(None, cx))),
                    )
                    .child(
                        button(p, "open-thread-number", "按串号打开串").on_click(
                            cx.listener(|this, _, _, cx| this.open_shared(Some(true), cx)),
                        ),
                    )
                    .child(
                        button(p, "open-message-id", "按 ID 浏览消息").on_click(
                            cx.listener(|this, _, _, cx| this.open_shared(Some(false), cx)),
                        ),
                    )
                    .child(button(p, "close-open-link", "取消").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.open_link_panel = false;
                            cx.notify();
                        },
                    ))),
            )
            .child(div().mt_2().text_xs().text_color(rgb(p.muted)).child(
                "粘贴岛内链接，或输入数字 / No.串号。回车打开链接；纯 ID 默认浏览单条消息。",
            ))
            .when_some(self.open_error.clone(), |d, error| {
                d.child(div().mt_2().child(error))
            })
            .into_any_element()
    }
}
