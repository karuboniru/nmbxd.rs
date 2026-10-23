// From:
// https://github.com/zed-industries/zed/blob/a8afc63a91f6b75528540dcffe73dc8ce0c92ad8/crates/gpui/examples/window_shadow.rs
use gpui::{
    AnyElement, App, Bounds, CursorStyle, Decorations, Edges, HitboxBehavior, Hsla,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement, Pixels, Point, RenderOnce,
    ResizeEdge, Size, Styled as _, Window, canvas, div, point, prelude::FluentBuilder as _, px,
};

use crate::ActiveTheme;

#[cfg(not(target_os = "linux"))]
const SHADOW_SIZE: Pixels = px(0.0);
#[cfg(target_os = "linux")]
const SHADOW_SIZE: Pixels = px(12.0);
const BORDER_SIZE: Pixels = px(1.0);
pub(crate) const BORDER_RADIUS: Pixels = px(0.0);

/// Create a new window border.
pub fn window_border() -> WindowBorder {
    WindowBorder::new()
}

/// Window border use to render a custom window border and shadow for Linux.
#[derive(IntoElement, Default)]
pub struct WindowBorder {
    children: Vec<AnyElement>,
}

impl WindowBorder {
    pub fn new() -> Self {
        Self {
            ..Default::default()
        }
    }
}

/// Get the window paddings.
pub fn window_paddings(window: &Window) -> Edges<Pixels> {
    match window.window_decorations() {
        Decorations::Server => Edges::all(px(0.0)),
        Decorations::Client { tiling } => {
            let mut paddings = Edges::all(SHADOW_SIZE);
            if tiling.top {
                paddings.top = px(0.0);
            }
            if tiling.bottom {
                paddings.bottom = px(0.0);
            }
            if tiling.left {
                paddings.left = px(0.0);
            }
            if tiling.right {
                paddings.right = px(0.0);
            }
            paddings
        }
    }
}

impl ParentElement for WindowBorder {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for WindowBorder {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let decorations = window.window_decorations();
        window.set_client_inset(SHADOW_SIZE);

        div()
            .id("window-backdrop")
            .bg(gpui::transparent_black())
            .map(|div| match decorations {
                Decorations::Server => div,
                Decorations::Client { tiling, .. } => div
                    .bg(gpui::transparent_black())
                    .child(
                        canvas(
                            |_bounds, window, _| {
                                window.insert_hitbox(
                                    Bounds::new(point(px(0.0), px(0.0)), window.viewport_size()),
                                    HitboxBehavior::Normal,
                                )
                            },
                            move |_bounds, hitbox, window, _| {
                                let mouse = window.mouse_position();
                                let Some(edge) = window_resize_edge(mouse, window) else {
                                    return;
                                };
                                window.set_cursor_style(
                                    match edge {
                                        ResizeEdge::Top | ResizeEdge::Bottom => {
                                            CursorStyle::ResizeUpDown
                                        }
                                        ResizeEdge::Left | ResizeEdge::Right => {
                                            CursorStyle::ResizeLeftRight
                                        }
                                        ResizeEdge::TopLeft | ResizeEdge::BottomRight => {
                                            CursorStyle::ResizeUpLeftDownRight
                                        }
                                        ResizeEdge::TopRight | ResizeEdge::BottomLeft => {
                                            CursorStyle::ResizeUpRightDownLeft
                                        }
                                    },
                                    &hitbox,
                                );
                            },
                        )
                        .size_full()
                        .absolute(),
                    )
                    .when(!(tiling.top || tiling.right), |div| {
                        div.rounded_tr(BORDER_RADIUS)
                    })
                    .when(!(tiling.top || tiling.left), |div| {
                        div.rounded_tl(BORDER_RADIUS)
                    })
                    .when(!tiling.top, |div| div.pt(SHADOW_SIZE))
                    .when(!tiling.bottom, |div| div.pb(SHADOW_SIZE))
                    .when(!tiling.left, |div| div.pl(SHADOW_SIZE))
                    .when(!tiling.right, |div| div.pr(SHADOW_SIZE))
                    .on_mouse_down(MouseButton::Left, move |_, window, _| {
                        let pos = window.mouse_position();

                        match window_resize_edge(pos, window) {
                            Some(edge) => window.start_window_resize(edge),
                            None => {}
                        };
                    }),
            })
            .size_full()
            .child(
                div()
                    .map(|div| match decorations {
                        Decorations::Server => div,
                        Decorations::Client { tiling } => div
                            .when(!(tiling.top || tiling.right), |div| {
                                div.rounded_tr(BORDER_RADIUS)
                            })
                            .when(!(tiling.top || tiling.left), |div| {
                                div.rounded_tl(BORDER_RADIUS)
                            })
                            .border_color(cx.theme().window_border)
                            .when(!tiling.top, |div| div.border_t(BORDER_SIZE))
                            .when(!tiling.bottom, |div| div.border_b(BORDER_SIZE))
                            .when(!tiling.left, |div| div.border_l(BORDER_SIZE))
                            .when(!tiling.right, |div| div.border_r(BORDER_SIZE))
                            .when(!tiling.is_tiled(), |div| {
                                div.shadow(vec![gpui::BoxShadow {
                                    color: Hsla {
                                        h: 0.,
                                        s: 0.,
                                        l: 0.,
                                        a: 0.3,
                                    },
                                    blur_radius: SHADOW_SIZE / 2.,
                                    spread_radius: px(0.),
                                    offset: point(px(0.0), px(0.0)),
                                }])
                            }),
                    })
                    .on_mouse_move(|_e, _, cx| {
                        cx.stop_propagation();
                    })
                    .bg(gpui::transparent_black())
                    .size_full()
                    .children(self.children),
            )
    }
}

fn window_resize_edge(pos: Point<Pixels>, window: &Window) -> Option<ResizeEdge> {
    if window.is_maximized() || window.is_fullscreen() {
        return None;
    }

    // Mouse coordinates use the current viewport. Maximized window_bounds can
    // contain the restored window's size, and tiled edges have no shadow gutter.
    resize_edge(pos, window_paddings(window), window.viewport_size())
}

fn resize_edge(
    pos: Point<Pixels>,
    paddings: Edges<Pixels>,
    size: Size<Pixels>,
) -> Option<ResizeEdge> {
    let top = paddings.top > px(0.) && pos.y < paddings.top;
    let bottom = paddings.bottom > px(0.) && pos.y > size.height - paddings.bottom;
    let left = paddings.left > px(0.) && pos.x < paddings.left;
    let right = paddings.right > px(0.) && pos.x > size.width - paddings.right;
    match (top, bottom, left, right) {
        (true, _, true, _) => Some(ResizeEdge::TopLeft),
        (true, _, _, true) => Some(ResizeEdge::TopRight),
        (true, _, _, _) => Some(ResizeEdge::Top),
        (_, true, true, _) => Some(ResizeEdge::BottomLeft),
        (_, true, _, true) => Some(ResizeEdge::BottomRight),
        (_, true, _, _) => Some(ResizeEdge::Bottom),
        (_, _, true, _) => Some(ResizeEdge::Left),
        (_, _, _, true) => Some(ResizeEdge::Right),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    #[test]
    fn floating_window_keeps_all_resize_edges() {
        let size = size(px(1000.), px(800.));
        let paddings = Edges::all(px(12.));
        for (x, y, edge) in [
            (1., 1., ResizeEdge::TopLeft),
            (500., 1., ResizeEdge::Top),
            (999., 1., ResizeEdge::TopRight),
            (1., 400., ResizeEdge::Left),
            (999., 400., ResizeEdge::Right),
            (1., 799., ResizeEdge::BottomLeft),
            (500., 799., ResizeEdge::Bottom),
            (999., 799., ResizeEdge::BottomRight),
        ] {
            assert_eq!(resize_edge(point(px(x), px(y)), paddings, size), Some(edge));
        }
        assert_eq!(resize_edge(point(px(500.), px(400.)), paddings, size), None);
    }

    #[test]
    fn tiled_edges_do_not_intercept_content() {
        let size = size(px(1920.), px(1080.));
        let mut paddings = Edges::all(px(12.));
        paddings.top = px(0.);
        paddings.right = px(0.);
        assert_eq!(resize_edge(point(px(1910.), px(10.)), paddings, size), None);
        assert_eq!(
            resize_edge(point(px(1.), px(10.)), paddings, size),
            Some(ResizeEdge::Left)
        );
        assert_eq!(
            resize_edge(point(px(1910.), px(1079.)), paddings, size),
            Some(ResizeEdge::Bottom)
        );
        assert_eq!(
            resize_edge(point(px(1919.), px(1.)), Edges::all(px(0.)), size),
            None
        );
    }
}
