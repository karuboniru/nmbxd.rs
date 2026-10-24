use crate::Island;
use gpui::{Context, KeystrokeEvent, MouseButton, MouseDownEvent, NavigationDirection, Window};

/// Close exactly one layer, keeping any underlying preview intact.
fn close_top_preview<T, I>(previews: &mut Vec<T>, viewing_image: &mut Option<I>) -> bool {
    previews.pop().is_some() || viewing_image.take().is_some()
}

fn is_preview_escape(event: &KeystrokeEvent) -> bool {
    event.keystroke.key == "escape"
        && !event
            .context_stack
            .iter()
            .any(|context| context.contains("PopupMenu"))
}

impl Island {
    /// Called from render; retain the subscription so registering once is enough.
    /// An interceptor also receives Escape when there is no focused element, or
    /// when a focused input handles the key before ordinary bubbling listeners.
    pub(crate) fn install_preview_shortcuts(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.preview_shortcut_subscription.is_some() {
            return;
        }

        let window_id = window.window_handle().window_id();
        let listener = cx.listener(move |this, event: &KeystrokeEvent, window, cx| {
            if window.window_handle().window_id() == window_id
                && is_preview_escape(event)
                && this.close_top_preview(cx)
            {
                cx.stop_propagation();
            }
        });
        self.preview_shortcut_subscription = Some(cx.intercept_keystrokes(listener));
    }

    /// Attach to the outer div with capture_any_mouse_down so children cannot
    /// consume the navigation button before the visible preview handles it.
    pub(crate) fn preview_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button == MouseButton::Navigate(NavigationDirection::Back)
            && self.close_top_preview(cx)
        {
            cx.stop_propagation();
        }
    }

    fn close_top_preview(&mut self, cx: &mut Context<Self>) -> bool {
        let closed = close_top_preview(&mut self.previews, &mut self.viewing_image)
            || (self.composing && std::mem::take(&mut self.kaomoji_open));
        if closed {
            cx.notify();
        }
        closed
    }
}

#[cfg(test)]
mod tests {
    use super::{close_top_preview, is_preview_escape};
    use gpui::{KeyContext, Keystroke, KeystrokeEvent};

    #[test]
    fn escape_dismisses_a_focused_menu_before_the_preview() {
        let mut event = KeystrokeEvent {
            keystroke: Keystroke::parse("escape").unwrap(),
            action: None,
            context_stack: vec![KeyContext::parse("PopupMenu").unwrap()],
        };
        let mut previews = vec![1, 2];
        let mut image = Some("original-image");
        if is_preview_escape(&event) {
            close_top_preview(&mut previews, &mut image);
        }
        assert_eq!(previews, vec![1, 2]);
        assert_eq!(image, Some("original-image"));

        // Once Cancel returns focus to the text, the next Escape closes one layer.
        event.context_stack.clear();
        assert!(is_preview_escape(&event));
        assert!(close_top_preview(&mut previews, &mut image));
        assert_eq!(previews, vec![1]);
        event.keystroke = Keystroke::parse("enter").unwrap();
        assert!(!is_preview_escape(&event));
    }

    #[test]
    fn closes_nested_messages_before_the_underlying_image() {
        let mut previews = vec![1, 2];
        let mut image = Some("original-image".to_owned());

        assert!(close_top_preview(&mut previews, &mut image));
        assert_eq!(previews, vec![1]);
        assert_eq!(image.as_deref(), Some("original-image"));

        assert!(close_top_preview(&mut previews, &mut image));
        assert!(previews.is_empty());
        assert_eq!(image.as_deref(), Some("original-image"));

        assert!(close_top_preview(&mut previews, &mut image));
        assert!(image.is_none());
        assert!(!close_top_preview(&mut previews, &mut image));
    }
}
