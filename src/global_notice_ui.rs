//! Startup announcement, with a durable opt-out for this exact announcement.
use crate::*;
use gpui_component::checkbox::Checkbox;
use rdnmb::global_notice::{GlobalNotice, SeenNotice};

pub(crate) struct NoticePanel {
    notice: GlobalNotice,
    seen: SeenNotice,
    checked: bool,
    error: Option<String>,
    content: Entity<selectable_content::SelectableContent>,
    focus: FocusHandle,
}

struct Close;
impl EventEmitter<Close> for NoticePanel {}

impl Island {
    pub(crate) fn check_global_notice(window: &mut Window, cx: &mut Context<Self>) {
        // Independent of the serialized forum worker: a slow notice server must
        // not delay opening the forum or loading replies.
        let task = cx.background_executor().spawn(async {
            let notice = rdnmb::global_notice::fetch_current()?;
            let Some(notice) = notice else {
                return Ok::<_, anyhow::Error>(None);
            };
            let (seen, error) = match SeenNotice::load() {
                Ok(seen) => (seen, None),
                Err(error) => (
                    SeenNotice::default(),
                    Some(format!("读取已读记录失败：{error}")),
                ),
            };
            Ok((!seen.is_seen(&notice)).then_some((notice, seen, error)))
        });
        cx.spawn_in(window, async move |view, cx| {
            let result = task.await;
            let _ = view.update_in(cx, |this, window, cx| match result {
                Ok(Some((notice, seen, error))) => {
                    let panel = cx.new(|cx| {
                        let focus = cx.focus_handle();
                        focus.focus(window);
                        let content = cx.new(|cx| {
                            selectable_content::SelectableContent::new(
                                rdnmb::content::parse_content(&notice.content),
                                |target, _, cx| match target {
                                    rdnmb::content::LinkTarget::External(url) => cx.open_url(url),
                                    rdnmb::content::LinkTarget::Reference(id) => {
                                        cx.open_url(&format!("{}t/{id}", rdnmb::api::WEBSITE_BASE))
                                    }
                                },
                                cx,
                            )
                        });
                        NoticePanel {
                            notice,
                            seen,
                            checked: false,
                            error,
                            content,
                            focus,
                        }
                    });
                    cx.subscribe(&panel, |this, _, _: &Close, cx| {
                        this.global_notice = None;
                        cx.notify();
                    })
                    .detach();
                    this.global_notice = Some(panel);
                    cx.notify();
                }
                Ok(None) => {}
                Err(error) => eprintln!("检查 X岛公告失败：{error}"),
            });
        })
        .detach();
    }
}

impl Render for NoticePanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let p = Palette::current(cx);
        div()
            .id("global-notice-overlay")
            .track_focus(&self.focus)
            .absolute()
            .inset_0()
            .occlude()
            .bg(rgba(0x00000088))
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(relative(0.8))
                    .max_w(px(760.))
                    .h(relative(0.75))
                    .flex()
                    .flex_col()
                    .bg(rgb(p.surface))
                    .border_1()
                    .border_color(rgb(p.border))
                    .rounded_lg()
                    .child(
                        div()
                            .p_4()
                            .flex_shrink_0()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("X岛公告"),
                    )
                    .child(
                        div()
                            .id("global-notice-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .p_4()
                            .text_size(px(16.))
                            .child(self.content.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .p_4()
                            .border_t_1()
                            .border_color(rgb(p.border))
                            .when_some(self.error.clone(), |d, error| {
                                d.child(div().mb_2().child(error))
                            })
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        Checkbox::new("global-notice-seen")
                                            .label("已读，下次不再显示这条公告")
                                            .checked(self.checked)
                                            .on_click(cx.listener(
                                                |this, checked: &bool, _, cx| {
                                                    match this.seen.set_seen(&this.notice, *checked)
                                                    {
                                                        Ok(()) => {
                                                            this.checked = *checked;
                                                            this.error = None;
                                                        }
                                                        Err(error) => {
                                                            this.error = Some(format!(
                                                                "保存已读记录失败：{error}"
                                                            ))
                                                        }
                                                    }
                                                    cx.notify();
                                                },
                                            )),
                                    )
                                    .child(
                                        Button::new("close-global-notice")
                                            .label("关闭")
                                            .on_click(cx.listener(|_, _, _, cx| cx.emit(Close))),
                                    ),
                            ),
                    ),
            )
    }
}
