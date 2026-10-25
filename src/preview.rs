//! On-demand, nested reference previews. Frames own request generations so
//! responses to closed previews can never reopen or replace a newer preview.
use crate::*;
use rdnmb::{
    api::ReferencePreview,
    content::{LinkTarget, parse_content},
};

#[derive(Clone, Copy, Default)]
struct ImageGeometry {
    viewport: Bounds<Pixels>,
    fitted: Size<Pixels>,
}

#[cfg(test)]
mod image_viewer_tests {
    use super::{ImageGeometry, ImageTransform, ImageViewer};
    use core::prelude::v1::test;
    use gpui::{Bounds, Point, point, px, size};

    fn geometry() -> ImageGeometry {
        ImageGeometry {
            viewport: Bounds::new(point(px(20.), px(30.)), size(px(400.), px(300.))),
            fitted: size(px(400.), px(300.)),
        }
    }

    #[test]
    fn zoom_preserves_the_image_coordinate_under_the_pointer() {
        let mut transform = ImageTransform::default();
        let geometry = geometry();
        let pointer = geometry.viewport.center() + point(px(40.), px(-20.));
        let before = (pointer - geometry.viewport.center() - transform.pan) / transform.zoom;
        transform.zoom_at(100., pointer, geometry);
        let after = (pointer - geometry.viewport.center() - transform.pan) / transform.zoom;
        assert!((f32::from(before.x - after.x)).abs() < 0.001);
        assert!((f32::from(before.y - after.y)).abs() < 0.001);
        assert!(transform.zoom > 1.);
    }

    #[test]
    fn zoom_limits_and_zooming_out_keep_the_image_visible() {
        let mut transform = ImageTransform::default();
        let geometry = geometry();
        for _ in 0..100 {
            transform.zoom_at(1000., geometry.viewport.center(), geometry);
        }
        assert_eq!(transform.zoom, 16.);
        transform.pan = point(px(100.), px(100.));
        for _ in 0..100 {
            transform.zoom_at(-1000., geometry.viewport.center(), geometry);
        }
        assert_eq!(transform.zoom, 0.1);
        assert_eq!(transform.pan, Point::default());
        transform.zoom_at(f32::NAN, geometry.viewport.center(), geometry);
        assert_eq!(transform.zoom, 0.1);
    }

    #[test]
    fn dragging_moves_both_axes_clamps_edges_and_stops_on_release() {
        let mut transform = ImageTransform {
            zoom: 2.,
            ..Default::default()
        };
        let geometry = geometry();
        transform.drag = Some(point(px(50.), px(60.)));
        transform.drag_to(point(px(80.), px(100.)), true, geometry);
        assert_eq!(transform.pan, point(px(30.), px(40.)));
        transform.drag_to(point(px(1000.), px(-1000.)), true, geometry);
        assert_eq!(transform.pan, point(px(200.), px(-150.)));
        transform.drag_to(point(px(50.), px(60.)), false, geometry);
        assert!(transform.drag.is_none());
        assert_eq!(transform.pan, point(px(200.), px(-150.)));
    }

    #[test]
    fn reopening_an_image_resets_zoom_and_pan() {
        let mut viewer = ImageViewer::new("first".into());
        viewer.transform.zoom = 3.;
        viewer.transform.pan = point(px(30.), px(40.));
        viewer.transform.drag = Some(Point::default());
        let viewer = ImageViewer::new(viewer.url);
        assert_eq!(viewer.transform.zoom, 1.);
        assert_eq!(viewer.transform.pan, Point::default());
        assert!(viewer.transform.drag.is_none());
    }
}

/// A fresh viewer owns its transform, so opening another image always starts fitted.
pub(crate) struct ImageViewer {
    pub(crate) url: String,
    transform: ImageTransform,
    geometry: std::rc::Rc<std::cell::Cell<ImageGeometry>>,
}

impl ImageViewer {
    pub(crate) fn new(url: String) -> Self {
        Self {
            url,
            transform: ImageTransform::default(),
            geometry: Default::default(),
        }
    }
}

struct ImageTransform {
    zoom: f32,
    pan: Point<Pixels>,
    drag: Option<Point<Pixels>>,
}

impl Default for ImageTransform {
    fn default() -> Self {
        Self {
            zoom: 1.,
            pan: Point::default(),
            drag: None,
        }
    }
}

impl ImageTransform {
    fn bounded_pan(&self, geometry: ImageGeometry) -> Point<Pixels> {
        let limit_x =
            ((geometry.fitted.width * self.zoom - geometry.viewport.size.width) / 2.).max(px(0.));
        let limit_y =
            ((geometry.fitted.height * self.zoom - geometry.viewport.size.height) / 2.).max(px(0.));
        point(
            self.pan.x.clamp(-limit_x, limit_x),
            self.pan.y.clamp(-limit_y, limit_y),
        )
    }

    fn zoom_at(&mut self, delta: f32, position: Point<Pixels>, geometry: ImageGeometry) {
        if !delta.is_finite() || delta == 0. {
            return;
        }
        let next = (self.zoom * (delta.clamp(-240., 240.) * 0.004).exp()).clamp(0.1, 16.);
        let ratio = next / self.zoom;
        let offset = position - geometry.viewport.center();
        // Keep the pixel under the pointer fixed until an image edge is reached.
        self.pan = offset - (offset - self.bounded_pan(geometry)) * ratio;
        self.zoom = next;
        self.pan = self.bounded_pan(geometry);
    }

    fn drag_to(&mut self, position: Point<Pixels>, dragging: bool, geometry: ImageGeometry) {
        if !dragging {
            self.drag = None;
            return;
        }
        if let Some(previous) = self.drag.replace(position) {
            self.pan = self.bounded_pan(geometry) + position - previous;
            self.pan = self.bounded_pan(geometry);
        }
    }
}

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
    pub(crate) fn image_overlay(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = Palette::current(cx);
        let viewer = self.viewing_image.as_ref().unwrap();
        let url = viewer.url.clone();
        let mut viewport = div()
            .id("image-viewport")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .rounded_md()
            .bg(rgb(p.background))
            .flex()
            .items_center()
            .justify_center();
        match self.media.read(cx).peek(&url) {
            Some(media::MediaState::Ready(image)) => {
                let geometry = viewer.geometry.clone();
                let zoom = viewer.transform.zoom;
                let pan = viewer.transform.pan;
                let entity = cx.entity().downgrade();
                viewport = viewport
                    .when(viewer.transform.drag.is_some(), |d| d.cursor_grabbing())
                    .when(viewer.transform.drag.is_none(), |d| d.cursor_grab())
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                        if let Some(viewer) = this.viewing_image.as_mut() {
                            viewer.transform.zoom_at(
                                event.delta.pixel_delta(px(20.)).y.into(),
                                event.position,
                                viewer.geometry.get(),
                            );
                            cx.stop_propagation();
                            cx.notify();
                        }
                    }))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| {
                            if let Some(viewer) = this.viewing_image.as_mut() {
                                viewer.transform.drag = Some(event.position);
                                cx.stop_propagation();
                                cx.notify();
                            }
                        }),
                    )
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, window, _| {
                                let natural = image.size(0);
                                let fit = (f32::from(bounds.size.width) / natural.width.0 as f32)
                                    .min(f32::from(bounds.size.height) / natural.height.0 as f32);
                                let fitted = size(
                                    px(natural.width.0 as f32 * fit),
                                    px(natural.height.0 as f32 * fit),
                                );
                                let current = ImageGeometry {
                                    viewport: bounds,
                                    fitted,
                                };
                                geometry.set(current);
                                let transform = ImageTransform {
                                    zoom,
                                    pan,
                                    drag: None,
                                };
                                let image_size = size(fitted.width * zoom, fitted.height * zoom);
                                let origin = bounds.center() + transform.bounded_pan(current)
                                    - point(image_size.width / 2., image_size.height / 2.);
                                let _ = window.paint_image(
                                    Bounds::new(origin, image_size),
                                    Corners::default(),
                                    image,
                                    0,
                                    false,
                                );

                                // Window listeners retain the drag when the pointer leaves
                                // the viewport, and release it even over the toolbar.
                                window.on_mouse_event({
                                    let entity = entity.clone();
                                    move |event: &MouseMoveEvent, phase, _, cx| {
                                        if !phase.bubble() {
                                            return;
                                        }
                                        let _ = entity.update(cx, |this, cx| {
                                            if let Some(viewer) = this.viewing_image.as_mut()
                                                && viewer.transform.drag.is_some()
                                            {
                                                viewer.transform.drag_to(
                                                    event.position,
                                                    event.dragging(),
                                                    viewer.geometry.get(),
                                                );
                                                cx.stop_propagation();
                                                cx.notify();
                                            }
                                        });
                                    }
                                });
                                window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                                    if !phase.bubble() || event.button != MouseButton::Left {
                                        return;
                                    }
                                    let _ = entity.update(cx, |this, cx| {
                                        if let Some(viewer) = this.viewing_image.as_mut()
                                            && viewer.transform.drag.take().is_some()
                                        {
                                            cx.stop_propagation();
                                            cx.notify();
                                        }
                                    });
                                });
                            },
                        )
                        .size_full(),
                    );
            }
            _ => {
                // Reuse the shared loading/error/retry UI until the image is ready.
                viewport = viewport.child(media_view::tile(&self.media, url.clone(), px(200.), cx));
            }
        }
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .p_4()
            .capture_any_mouse_down(cx.listener(Self::preview_mouse_down))
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_sm()
                            .child("原图 · 滚轮缩放，拖动平移 · 动图暂显示首帧"),
                    )
                    .child(self.image_actions("viewer-image-actions", url, cx))
                    .child(button(p, "close-image", "关闭原图").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.viewing_image = None;
                            cx.notify();
                        },
                    ))),
            )
            .child(viewport)
            .into_any_element()
    }

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
                if let Some(original) = value.post.image_url(false) {
                    let image_url = original.clone();
                    body = body.child(
                        div()
                            .id(("preview-image", generation))
                            .my_2()
                            .cursor_pointer()
                            .child(media_view::tile(&self.media, url, px(240.), cx))
                            .child(
                                div()
                                    .mt_1()
                                    .text_xs()
                                    .text_color(rgb(p.muted))
                                    .child("点击查看原图"),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.viewing_image = Some(ImageViewer::new(image_url.clone()));
                                cx.notify();
                            })),
                    );
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
