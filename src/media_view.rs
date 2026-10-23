//! Visible-only image request boundary shared by thumbnails and the full viewer.
use crate::{
    appearance::Palette,
    media::{MediaState, MediaStore},
};
use gpui::{prelude::*, *};

pub fn tile(store: &Entity<MediaStore>, url: String, height: Pixels, cx: &App) -> AnyElement {
    let p = Palette::current(cx);
    let state = store.read(cx).peek(&url);
    let mut tile = div()
        .relative()
        .w_full()
        .h(height)
        .overflow_hidden()
        .rounded_md()
        .bg(rgb(p.background))
        .flex()
        .items_center()
        .justify_center();
    match state {
        Some(MediaState::Ready(image)) => {
            tile = tile.child(img(image).size_full().object_fit(ObjectFit::Contain));
        }
        Some(MediaState::Failed(error)) => {
            let retry_store = store.clone();
            let retry_url = url.clone();
            tile = tile.child(
                div()
                    .id(SharedString::from(format!("retry-{url}")))
                    .p_3()
                    .text_xs()
                    .text_color(rgb(p.muted))
                    .cursor_pointer()
                    .child(format!("图片加载失败：{error}\n点击重试"))
                    .on_click(move |_, _, cx| {
                        cx.stop_propagation();
                        retry_store.update(cx, |store, cx| {
                            store.retry(&retry_url, cx);
                        });
                    }),
            );
        }
        _ => {
            tile = tile.child(
                div()
                    .text_xs()
                    .text_color(rgb(p.muted))
                    .child("图片加载中…"),
            );
        }
    }
    let store = store.clone();
    tile.child(
        canvas(
            |_, _, _| (),
            move |bounds, _, window, cx| {
                let visible = bounds.intersect(&window.content_mask().bounds);
                if visible.size.width > px(0.) && visible.size.height > px(0.) {
                    store.update(cx, |store, cx| {
                        store.load(&url, cx);
                    });
                }
            },
        )
        .absolute()
        .size_full(),
    )
    .into_any_element()
}
