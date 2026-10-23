//! Explicit copy / save operations, independent of posting and draft state.
use crate::*;

impl Island {
    pub(crate) fn copy_text_button(
        &self,
        id: impl Into<ElementId>,
        html: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let text = rdnmb::content::parse_content(html).text;
        button(Palette::current(cx), id, "复制正文")
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                cx.write_to_clipboard(ClipboardItem::new_string(text.clone()));
                this.notice = "正文已复制".into();
                cx.notify();
            }))
            .into_any_element()
    }

    pub(crate) fn image_actions(
        &self,
        id: impl Into<ElementId>,
        url: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = Palette::current(cx);
        let download = url.clone();
        div()
            .id(id)
            .flex()
            .flex_shrink_0()
            .whitespace_nowrap()
            .gap_2()
            .my_2()
            .when(self.image_export_busy, |d| d.opacity(0.5))
            .child(button(p, "copy-image", "复制图片").on_click(cx.listener(
                move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.copy_image(url.clone(), cx);
                },
            )))
            .child(button(p, "save-image", "图片另存为…").on_click(cx.listener(
                move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.save_image(download.clone(), window, cx);
                },
            )))
            .into_any_element()
    }

    fn copy_image(&mut self, url: String, cx: &mut Context<Self>) {
        if self.image_export_busy {
            return;
        }
        self.image_export_busy = true;
        self.notice = "正在准备原图，复制为 PNG…".into();
        let task = cx.background_executor().spawn(async move {
            let image = image_export::fetch(&url)?;
            image_clipboard::write(image.clipboard)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.image_export_busy = false;
                match result {
                    Ok(()) => {
                        this.notice = "图片已复制（PNG，动图取首帧）".into();
                    }
                    Err(error) => this.notice = format!("复制图片失败：{error}"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn save_image(&mut self, url: String, window: &mut Window, cx: &mut Context<Self>) {
        if self.image_export_busy {
            return;
        }
        self.image_export_busy = true;
        self.notice = "正在准备原图，随后请选择保存路径…".into();
        let task = cx
            .background_executor()
            .spawn(async move { image_export::fetch(&url) });
        cx.spawn_in(window, async move |this, cx| {
            let result = async {
                let image = task.await?;
                let picker = this
                    .update_in(cx, |_, window, _| {
                        image_export::pick_save(window, image.filename)
                    })
                    .map_err(|_| "保存窗口已关闭".to_owned())?;
                let Some(path) = picker.await? else {
                    return Ok(false);
                };
                cx.background_executor()
                    .spawn(async move { image_export::save(path, image.bytes) })
                    .await?;
                Ok::<_, String>(true)
            }
            .await;
            let _ = this.update(cx, |this, cx| {
                this.image_export_busy = false;
                this.notice = match result {
                    Ok(true) => "原图已保存".into(),
                    Ok(false) => "已取消保存图片".into(),
                    Err(error) => format!("保存图片失败：{error}"),
                };
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
