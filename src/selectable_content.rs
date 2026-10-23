//! Plain post text with persistent selection and the same links as the parser.
//!
//! Keep this entity alive between renders: selection belongs to the entity,
//! while each frame's text element only supplies layout and mouse hit testing.
use crate::appearance::Palette;
use gpui::{prelude::*, *};
use nmbxd::content::{ContentLink, LinkTarget, ParsedContent};
use std::{ops::Range, rc::Rc};

type LinkCallback = Rc<dyn Fn(&LinkTarget, &mut Window, &mut App)>;

struct Press {
    index: usize,
    position: Point<Pixels>,
    link: Option<usize>,
    dragged: bool,
}

pub(crate) struct SelectableContent {
    text: SharedString,
    links: Rc<Vec<ContentLink>>,
    selection: Range<usize>,
    press: Option<Press>,
    focus: FocusHandle,
    on_link: LinkCallback,
}

impl SelectableContent {
    pub(crate) fn new(
        parsed: ParsedContent,
        on_link: impl Fn(&LinkTarget, &mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            text: parsed.text.into(),
            links: Rc::new(parsed.links),
            selection: 0..0,
            press: None,
            focus: cx.focus_handle(),
            on_link: Rc::new(on_link),
        }
    }

    pub(crate) fn retention_priority(&self) -> u8 {
        if self.press.is_some() {
            2
        } else if self.selection.start != self.selection.end {
            1
        } else {
            0
        }
    }

    fn copy(&self, cx: &mut App) {
        let range = ordered_selection(&self.selection);
        if !range.is_empty() {
            // Preserve the exact selected whitespace and original line breaks.
            cx.write_to_clipboard(ClipboardItem::new_string(self.text[range].to_owned()));
        }
    }
}

impl Render for SelectableContent {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let palette = Palette::current(cx);
        let color = rgb(palette.accent).into();
        let text =
            StyledText::new(self.text.clone()).with_highlights(self.links.iter().map(|link| {
                (
                    link.range.clone(),
                    HighlightStyle {
                        color: Some(color),
                        ..Default::default()
                    },
                )
            }));
        div()
            .id("selectable-content")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let modifiers = event.keystroke.modifiers;
                if !modifiers.secondary() || modifiers.alt {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "c" => {
                        this.copy(cx);
                        cx.stop_propagation();
                    }
                    "a" => {
                        this.selection = 0..this.text.len();
                        this.press = None;
                        cx.notify();
                        cx.stop_propagation();
                    }
                    _ => {}
                }
            }))
            .child(SelectableText {
                text,
                value: self.text.clone(),
                links: self.links.clone(),
                selection: ordered_selection(&self.selection),
                selection_color: rgb(palette.selected).into(),
                entity: cx.entity().downgrade(),
            })
    }
}

struct SelectableText {
    text: StyledText,
    value: SharedString,
    links: Rc<Vec<ContentLink>>,
    selection: Range<usize>,
    selection_color: Hsla,
    entity: WeakEntity<SelectableContent>,
}

impl IntoElement for SelectableText {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl Element for SelectableText {
    type RequestLayoutState = ();
    type PrepaintState = Hitbox;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        // StyledText inherits the surrounding post's font, size and line height.
        self.text.request_layout(id, inspector, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Hitbox {
        self.text.prepaint(id, inspector, bounds, state, window, cx);
        window.insert_hitbox(bounds, HitboxBehavior::Normal)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut (),
        hitbox: &mut Hitbox,
        window: &mut Window,
        cx: &mut App,
    ) {
        let layout = self.text.layout().clone();
        paint_selection(&layout, &self.selection, self.selection_color, window);
        self.text
            .paint(id, inspector, bounds, state, &mut (), window, cx);

        let hovered_link = layout
            .index_for_position(window.mouse_position())
            .ok()
            .and_then(|index| link_at(&self.links, index));
        window.set_cursor_style(
            if hovered_link.is_some() {
                CursorStyle::PointingHand
            } else {
                CursorStyle::IBeam
            },
            hitbox,
        );

        window.on_mouse_event({
            let entity = self.entity.clone();
            let hitbox = hitbox.clone();
            let layout = layout.clone();
            let value = self.value.clone();
            move |event: &MouseDownEvent, phase, window, cx| {
                if !phase.bubble()
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                let index = nearest_index(&layout, &value, event.position);
                let _ = entity.update(cx, |this, cx| {
                    this.focus.focus(window);
                    this.selection = index..index;
                    this.press = Some(Press {
                        index,
                        position: event.position,
                        link: layout
                            .index_for_position(event.position)
                            .ok()
                            .and_then(|index| link_at(&this.links, index)),
                        dragged: false,
                    });
                    cx.notify();
                });
            }
        });

        window.on_mouse_event({
            let entity = self.entity.clone();
            let layout = layout.clone();
            let value = self.value.clone();
            let hitbox = hitbox.clone();
            let links = self.links.clone();
            let mut previous_link = hovered_link;
            move |event: &MouseMoveEvent, phase, window, cx| {
                if !phase.bubble() {
                    return;
                }
                let hovered_link = if hitbox.is_hovered(window) {
                    layout
                        .index_for_position(event.position)
                        .ok()
                        .and_then(|index| link_at(&links, index))
                } else {
                    None
                };
                if hovered_link != previous_link {
                    previous_link = hovered_link;
                    window.refresh();
                }
                if !event.dragging() {
                    return;
                }
                let index = nearest_index(&layout, &value, event.position);
                let _ = entity.update(cx, |this, cx| {
                    let Some(press) = &mut this.press else { return };
                    press.dragged |= moved(press.position, event.position) || index != press.index;
                    this.selection.end = index;
                    cx.notify();
                    cx.stop_propagation();
                });
            }
        });

        window.on_mouse_event({
            let entity = self.entity.clone();
            let hitbox = hitbox.clone();
            let value = self.value.clone();
            move |event: &MouseUpEvent, phase, window, cx| {
                if !phase.bubble() || event.button != MouseButton::Left {
                    return;
                }
                let index = nearest_index(&layout, &value, event.position);
                let result = entity.update(cx, |this, cx| {
                    let press = this.press.take()?;
                    this.selection.end = index;
                    let dragged = press.dragged
                        || moved(press.position, event.position)
                        || index != press.index;
                    let link = if !dragged && hitbox.is_hovered(window) {
                        layout
                            .index_for_position(event.position)
                            .ok()
                            .and_then(|index| link_at(&this.links, index))
                            .filter(|index| Some(*index) == press.link)
                            .map(|index| this.links[index].target.clone())
                    } else {
                        None
                    };
                    if dragged || link.is_some() {
                        cx.stop_propagation();
                    }
                    cx.notify();
                    link.map(|target| (target, this.on_link.clone()))
                });
                if let Ok(Some((target, on_link))) = result {
                    on_link(&target, window, cx);
                }
            }
        });
    }
}

fn link_at(links: &[ContentLink], index: usize) -> Option<usize> {
    links.iter().position(|link| link.range.contains(&index))
}

fn moved(start: Point<Pixels>, end: Point<Pixels>) -> bool {
    let delta = end - start;
    delta.x > px(3.) || delta.x < px(-3.) || delta.y > px(3.) || delta.y < px(-3.)
}

fn nearest_index(layout: &TextLayout, text: &str, position: Point<Pixels>) -> usize {
    let index = match layout.index_for_position(position) {
        Ok(index) | Err(index) => index,
    };
    char_boundary(text, index)
}

fn char_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn ordered_selection(selection: &Range<usize>) -> Range<usize> {
    selection.start.min(selection.end)..selection.start.max(selection.end)
}

fn paint_selection(
    layout: &TextLayout,
    selection: &Range<usize>,
    color: Hsla,
    window: &mut Window,
) {
    if selection.is_empty() {
        return;
    }
    let (Some(start), Some(end)) = (
        layout.position_for_index(selection.start),
        layout.position_for_index(selection.end),
    ) else {
        return;
    };
    let line_height = layout.line_height();
    let bounds = layout.bounds();
    let mut paint = |start, end| {
        let bounds = Bounds::from_corners(start, end);
        if bounds.size.width > px(0.) && bounds.size.height > px(0.) {
            window.paint_quad(quad(
                bounds,
                px(0.),
                color,
                Edges::default(),
                transparent_black(),
                BorderStyle::default(),
            ));
        }
    };
    if start.y == end.y {
        paint(start, point(end.x, end.y + line_height));
    } else {
        paint(start, point(bounds.right(), start.y + line_height));
        paint(
            point(bounds.left(), start.y + line_height),
            point(bounds.right(), end.y),
        );
        paint(
            point(bounds.left(), end.y),
            point(end.x, end.y + line_height),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{char_boundary, ordered_selection};

    #[test]
    fn unicode_selection_uses_utf8_boundaries_and_keeps_whitespace() {
        let text = " 岛😀\n正文 ";
        assert_eq!(char_boundary(text, 3), 1);
        assert_eq!(char_boundary(text, 6), 4);
        assert_eq!(char_boundary(text, usize::MAX), text.len());
        let selection = ordered_selection(&(text.len()..1));
        assert_eq!(&text[selection], "岛😀\n正文 ");
        assert_eq!(&text[ordered_selection(&(0..text.len()))], text);
    }
}
