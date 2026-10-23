//! On-demand, nested reference previews. Frames own request generations so
//! responses to closed previews can never reopen or replace a newer preview.
use crate::*;
use rdnmb::{
    api::ReferencePreview,
    content::{LinkTarget, parse_content},
};

pub(crate) struct Frame {
    lifetime: std::sync::Arc<()>,
    generation: u64,
    id: u64,
    value: Option<ReferencePreview>,
    error: Option<String>,
    busy: bool,
    scroll: ScrollHandle,
}

impl Island {
    pub(crate) fn rich_content(
        &self,
        id: impl Into<ElementId>,
        html: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = id.into();
        let mut cache = self.content_views.borrow_mut();
        if let Some(index) = cache
            .iter()
            .position(|(key, source, _)| *key == id && source == html)
        {
            let entry = cache.remove(index).unwrap();
            let entity = entry.2.clone();
            cache.push_back(entry);
            return entity.into_any_element();
        }
        cache.retain(|(key, _, _)| *key != id);
        let parsed = parse_content(html);
        let view = cx.entity().downgrade();
        let entity = cx.new(|cx| {
            selectable_content::SelectableContent::new(
                parsed,
                move |target, _, cx| match target {
                    LinkTarget::External(url) => cx.open_url(url),
                    LinkTarget::Reference(id) => {
                        let _ = view.update(cx, |this, cx| this.open_preview(*id, cx));
                    }
                },
                cx,
            )
        });
        cache.push_back((id, html.to_owned(), entity.clone()));
        while cache.len() > 128 {
            // Protect an active drag/selection when a buffered tree contains
            // more posts than the cache; otherwise each render evicts it.
            let index = cache
                .iter()
                .enumerate()
                .min_by_key(|(_, (_, _, view))| view.read(cx).retention_priority())
                .map(|(index, _)| index)
                .unwrap();
            cache.remove(index);
        }
        entity.into_any_element()
    }

    fn local_reference(&self, id: u64) -> Option<ReferencePreview> {
        if let Some(tree) = &self.tree
            && let Some(node) = tree.nodes.get(&id)
        {
            return Some(ReferencePreview {
                post: node.post.clone(),
                thread_id: Some(tree.root_id),
            });
        }
        for root in &self.threads {
            if root.id == id {
                return Some(ReferencePreview {
                    post: root.clone(),
                    thread_id: Some(id),
                });
            }
            if let Some(post) = root.replies.iter().find(|post| post.id == id) {
                return Some(ReferencePreview {
                    post: post.clone(),
                    thread_id: Some(root.id),
                });
            }
        }
        None
    }

    fn open_preview(&mut self, id: u64, cx: &mut Context<Self>) {
        if let Some(index) = self.previews.iter().position(|frame| frame.id == id) {
            self.previews.truncate(index + 1);
            cx.notify();
            return;
        }
        if self.previews.len() >= 32 {
            self.notice = "预览最多嵌套 32 层，请先返回上一层".into();
            cx.notify();
            return;
        }
        let value = self.local_reference(id);
        self.preview_serial += 1;
        let generation = self.preview_serial;
        let busy = value.is_none();
        let lifetime = std::sync::Arc::new(());
        let request_lifetime = std::sync::Arc::downgrade(&lifetime);
        self.previews.push(Frame {
            lifetime,
            generation,
            id,
            value,
            error: None,
            busy,
            scroll: ScrollHandle::new(),
        });
        if busy
            && self
                .tx
                .send(Request::Reference(generation, id, request_lifetime))
                .is_err()
        {
            self.receive_preview(generation, Err("请求服务已停止".into()));
        }
        cx.notify();
    }

    pub(crate) fn receive_preview(
        &mut self,
        generation: u64,
        result: Result<ReferencePreview, String>,
    ) {
        if let Some(frame) = self
            .previews
            .iter_mut()
            .find(|f| f.generation == generation)
        {
            frame.busy = false;
            match result {
                Ok(value) => {
                    frame.value = Some(value);
                    frame.error = None;
                }
                Err(error) => frame.error = Some(error),
            }
        }
    }

    fn jump_preview(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = self.previews.last() else {
            return;
        };
        if frame.busy || frame.value.is_none() {
            return;
        }
        let id = frame.id;
        let root = self
            .local_reference(id)
            .and_then(|v| v.thread_id)
            .or_else(|| frame.value.as_ref().and_then(|v| v.thread_id));
        // A current-thread reference is a true noop: retain scroll, pages,
        // aggregation state, draft, and even the preview stack.
        if root == self.selected && root.is_some() {
            return;
        }
        if self.navigation.loading() {
            self.previews.last_mut().unwrap().error = Some("当前串正在加载，请稍后跳转".into());
        } else if let Some(root) = root {
            self.open_thread(root, cx);
            self.previews.clear();
        } else {
            let frame = self.previews.last_mut().unwrap();
            frame.busy = true;
            frame.error = None;
            if self
                .tx
                .send(Request::JumpProbe(
                    frame.generation,
                    id,
                    std::sync::Arc::downgrade(&frame.lifetime),
                ))
                .is_err()
            {
                frame.busy = false;
                frame.error = Some("请求服务已停止".into());
            }
        }
        cx.notify();
    }

    pub(crate) fn receive_jump(
        &mut self,
        generation: u64,
        result: Result<Post, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self
            .previews
            .iter()
            .position(|f| f.generation == generation)
        else {
            return;
        };
        self.previews[index].busy = false;
        match result {
            Ok(post) => {
                if let Some(value) = &mut self.previews[index].value {
                    value.thread_id = Some(post.id);
                }
                // Do not navigate for an older frame hidden beneath a newer one.
                if index + 1 != self.previews.len() || self.selected == Some(post.id) {
                    return;
                }
                if self.navigation.loading() {
                    self.previews[index].error = Some("当前串正在加载，请稍后跳转".into());
                    return;
                }
                let id = post.id;
                if let Some(store) = &self.history {
                    let _ = store.cache_page(id, 1, &post);
                }
                self.open_thread(id, cx);
                self.previews.clear();
            }
            Err(error) => {
                self.previews[index].error = Some(format!(
                    "无法跳转：{error}。引用接口不提供所属主串；若这是串外回复，暂无法定位主串。"
                ))
            }
        }
        cx.notify();
    }

    pub(crate) fn preview_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = Palette::current(cx);
        let frame = self.previews.last().unwrap();
        let generation = frame.generation;
        let current = self
            .local_reference(frame.id)
            .and_then(|v| v.thread_id)
            .or_else(|| frame.value.as_ref().and_then(|v| v.thread_id))
            == self.selected
            && self.selected.is_some();
        let mut body = div()
            .id(("preview-scroll", generation))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&frame.scroll)
            .p_4();
        if let Some(value) = &frame.value {
            body = body
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(p.muted))
                        .child(format!("{} · {}", value.post.user_hash, value.post.now)),
                )
                .child(
                    div()
                        .my_3()
                        .text_size(px(self.settings.body_font_size))
                        .child(value.post.title.clone()),
                )
                .child(
                    div()
                        .text_size(px(self.settings.body_font_size))
                        .child(self.rich_content(
                            ("preview-body", generation),
                            &value.post.content,
                            cx,
                        )),
                );
            body = body.child(self.copy_text_button(
                ("copy-preview", generation),
                &value.post.content,
                cx,
            ));
            if let Some(url) = value.post.image_url(true) {
                body = body.child(media_view::tile(&self.media, url, px(240.), cx));
                if let Some(original) = value.post.image_url(false) {
                    body = body.child(self.image_actions(
                        ("preview-image-actions", generation),
                        original,
                        cx,
                    ));
                }
            }
        }
        if frame.busy {
            body = body.child(div().mt_3().child("正在加载…"));
        }
        if let Some(error) = &frame.error {
            body = body.child(div().mt_3().child(error.clone()));
            if frame.value.is_none() {
                body = body.child(button(p, "retry-preview", "重试").on_click(cx.listener(
                    |this, _, _, cx| {
                        if let Some(frame) = this.previews.last_mut()
                            && !frame.busy
                        {
                            frame.busy = true;
                            frame.error = None;
                            if this
                                .tx
                                .send(Request::Reference(
                                    frame.generation,
                                    frame.id,
                                    std::sync::Arc::downgrade(&frame.lifetime),
                                ))
                                .is_err()
                            {
                                frame.busy = false;
                                frame.error = Some("请求服务已停止".into());
                            }
                        }
                        cx.notify();
                    },
                )));
            }
        }
        div()
            .absolute()
            .inset_0()
            .bg(rgba(0x00000088))
            .flex()
            .items_center()
            .justify_center()
            .occlude()
            .capture_any_mouse_down(cx.listener(Self::preview_mouse_down))
            .child(
                div()
                    .w(relative(0.8))
                    .h(relative(0.8))
                    .max_w(px(900.))
                    .flex()
                    .flex_col()
                    .bg(rgb(p.surface))
                    .border_1()
                    .border_color(rgb(p.border))
                    .rounded_lg()
                    .child(
                        div()
                            .p_3()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(button(p, "preview-back", "返回上层").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.previews.pop();
                                    cx.notify();
                                },
                            )))
                            .child(div().flex_1().child(format!(
                                "引用预览 No.{} · 第 {} 层",
                                frame.id,
                                self.previews.len()
                            )))
                            .child(
                                button(
                                    p,
                                    "preview-jump",
                                    if current {
                                        "已在当前串"
                                    } else {
                                        "跳转到串"
                                    },
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.jump_preview(cx))),
                            )
                            .child(button(p, "preview-close", "关闭预览").on_click(cx.listener(
                                |this, _, _, cx| {
                                    this.previews.clear();
                                    cx.notify();
                                },
                            ))),
                    )
                    .child(body),
            )
            .into_any_element()
    }
}
