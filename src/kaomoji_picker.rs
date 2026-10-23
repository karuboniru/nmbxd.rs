//! A bounded, scrollable picker shared by new-thread and reply drafts.
use crate::*;

impl Island {
    pub(crate) fn kaomoji_picker(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = Palette::current(cx);
        div()
            .mx_4()
            .my_2()
            .p_2()
            .h(px(180.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .border_1()
            .border_color(rgb(p.border))
            .rounded_md()
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(p.muted))
                    .child("网页版颜文字 · 点击插入正文 · Esc 收起"),
            )
            .child(
                div()
                    .id("kaomoji-options")
                    .mt_2()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(div().flex().flex_wrap().gap_1().children(
                        kaomoji::KAOMOJI.iter().enumerate().map(|(index, entry)| {
                            let text = entry.value;
                            button(p, ("kaomoji", index), entry.label)
                                .text_size(px(self.settings.body_font_size))
                                .when(self.sending, |d| d.opacity(0.4))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    if this.sending {
                                        return;
                                    }
                                    this.body.update(cx, |input, cx| {
                                        // Use the input's native replacement path: selection,
                                        // Unicode cursor offsets, history and change events.
                                        input.replace(text, window, cx);
                                        input.focus(window, cx);
                                    });
                                    this.kaomoji_open = false;
                                    cx.notify();
                                }))
                        }),
                    )),
            )
            .with_animation(
                "kaomoji-enter",
                Animation::new(Duration::from_millis(150)).with_easing(ease_out_quint()),
                |d, delta| d.opacity(delta),
            )
            .into_any_element()
    }
}
