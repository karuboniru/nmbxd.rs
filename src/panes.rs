//! Session-local pane widths and mouse-driven separators.
use std::{cell::Cell, rc::Rc};

use gpui::{prelude::*, *};
use gpui_component::PixelsExt;

use crate::appearance::Palette;

const HANDLE: f32 = 6.;
const MINIMUMS: [f32; 4] = [120., 200., 300., 240.];

pub struct PaneLayout {
    state: Entity<PaneState>,
}

impl PaneLayout {
    pub fn new<T: 'static>(cx: &mut Context<T>) -> Self {
        let state = cx.new(|_| PaneState::default());
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self { state }
    }

    pub fn render(
        &self,
        sidebar: AnyElement,
        threads: AnyElement,
        reader: AnyElement,
        composer: Option<AnyElement>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let composing = composer.is_some();
        let widths = self.state.update(cx, |state, _| {
            let width = if state.measured_width > 0. {
                state.measured_width
            } else {
                window.viewport_size().width.as_f32()
            };
            state.sizes.prepare(width, composing);
            state.sizes.widths
        });
        let count = if composing { 4 } else { 3 };
        let row_width = widths[..count].iter().sum::<f32>() + HANDLE * (count - 1) as f32;
        let mut row = div().flex().h_full().w(px(row_width)).flex_shrink_0();
        let contents = [Some(sidebar), Some(threads), Some(reader), composer];
        for (index, content) in contents.into_iter().enumerate().take(count) {
            if index > 0 {
                row = row.child(self.separator(index - 1, widths, cx));
            }
            row = row.child(
                div()
                    .w(px(widths[index]))
                    .h_full()
                    .min_w_0()
                    .flex_shrink_0()
                    .overflow_hidden()
                    .child(content.unwrap()),
            );
        }
        let state = self.state.clone();
        div()
            .id("pane-columns")
            .relative()
            .flex_1()
            .w_full()
            .min_w_0()
            .min_h_0()
            .overflow_x_scroll()
            .child(row)
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let width = bounds.size.width.as_f32();
                        state.update(cx, |state, cx| {
                            if (state.measured_width - width).abs() > 0.5 {
                                state.measured_width = width;
                                cx.notify();
                            }
                        });
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .into_any_element()
    }

    fn separator(&self, index: usize, widths: [f32; 4], cx: &App) -> AnyElement {
        let p = Palette::current(cx);
        let drag = PaneDrag {
            state: self.state.clone(),
            index,
            widths,
            start_x: Rc::new(Cell::new(0.)),
        };
        div()
            .id(("pane-separator", index))
            .w(px(HANDLE))
            .h_full()
            .flex_shrink_0()
            .flex()
            .justify_center()
            .cursor_col_resize()
            .bg(rgb(p.background))
            .hover(move |style| style.bg(rgb(p.selected)))
            .child(div().w(px(1.)).h_full().bg(rgb(p.border)))
            .on_drag(drag, |drag, _, window, cx| {
                drag.start_x.set(window.mouse_position().x.as_f32());
                cx.stop_propagation();
                cx.new(|_| EmptyView)
            })
            .on_drag_move(move |event: &DragMoveEvent<PaneDrag>, _, cx| {
                let drag = event.drag(cx);
                if drag.index != index {
                    return;
                }
                let state = drag.state.clone();
                let index = drag.index;
                let widths = drag.widths;
                let delta = event.event.position.x.as_f32() - drag.start_x.get();
                state.update(cx, |state, cx| {
                    // The drag's starting widths prevent rounding and pointer drift.
                    state.sizes.resize(index, widths, delta);
                    cx.notify();
                });
            })
            .into_any_element()
    }
}

#[derive(Default)]
struct PaneState {
    measured_width: f32,
    sizes: PaneSizes,
}

struct PaneDrag {
    state: Entity<PaneState>,
    index: usize,
    widths: [f32; 4],
    start_x: Rc<Cell<f32>>,
}

#[derive(Debug)]
struct PaneSizes {
    widths: [f32; 4],
    container: f32,
    initialized: bool,
    composing: bool,
}

impl Default for PaneSizes {
    fn default() -> Self {
        Self {
            widths: [170., 290., 300., 320.],
            container: 0.,
            initialized: false,
            composing: false,
        }
    }
}

impl PaneSizes {
    fn prepare(&mut self, container: f32, composing: bool) {
        let count = if composing { 4 } else { 3 };
        let available =
            (container - HANDLE * (count - 1) as f32).max(MINIMUMS[..count].iter().sum());
        if !self.initialized {
            self.widths[2] = (available
                - self.widths[0]
                - self.widths[1]
                - if composing { self.widths[3] } else { 0. })
            .max(MINIMUMS[2]);
        } else if composing != self.composing {
            // Closing returns the editor's space to reading; reopening restores
            // its last width without resetting the other panes.
            if composing {
                self.widths[2] -= self.widths[3] + HANDLE;
            } else {
                self.widths[2] += self.widths[3] + HANDLE;
            }
        } else if (container - self.container).abs() < 0.5 {
            return;
        }
        fit(&mut self.widths, count, available);
        self.container = container;
        self.composing = composing;
        self.initialized = true;
    }

    fn resize(&mut self, index: usize, starting: [f32; 4], delta: f32) {
        let count = if self.composing { 4 } else { 3 };
        if index + 1 >= count {
            return;
        }
        let delta = delta.clamp(
            MINIMUMS[index] - starting[index],
            starting[index + 1] - MINIMUMS[index + 1],
        );
        self.widths[index] = starting[index] + delta;
        self.widths[index + 1] = starting[index + 1] - delta;
    }
}

/// Keep the current proportions while enforcing each visible pane's minimum.
fn fit(widths: &mut [f32; 4], count: usize, available: f32) {
    let weights = widths.map(|width| width.max(1.));
    let mut pending = [true; 4];
    let mut remaining = available;
    loop {
        let total: f32 = (0..count).filter(|&i| pending[i]).map(|i| weights[i]).sum();
        if total == 0. {
            break;
        }
        let scale = remaining / total;
        let mut clamped = false;
        for i in 0..count {
            if pending[i] && scale * weights[i] < MINIMUMS[i] {
                widths[i] = MINIMUMS[i];
                remaining -= MINIMUMS[i];
                pending[i] = false;
                clamped = true;
            }
        }
        if !clamped {
            for i in 0..count {
                if pending[i] {
                    widths[i] = remaining * weights[i] / total;
                }
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::prelude::v1::test;

    fn close(left: f32, right: f32) {
        assert!((left - right).abs() < 0.01, "{left} != {right}");
    }

    #[test]
    fn defaults_leave_remaining_space_for_reading() {
        let mut sizes = PaneSizes::default();
        sizes.prepare(1280., false);
        close(sizes.widths[0], 170.);
        close(sizes.widths[1], 290.);
        close(sizes.widths[2], 808.);
    }

    #[test]
    fn dragging_clamps_both_adjacent_panes_and_preserves_total() {
        let mut sizes = PaneSizes::default();
        sizes.prepare(1280., true);
        let start = sizes.widths;
        sizes.resize(1, start, 10000.);
        close(sizes.widths[2], MINIMUMS[2]);
        close(sizes.widths[1] + sizes.widths[2], start[1] + start[2]);
        sizes.resize(1, start, -10000.);
        close(sizes.widths[1], MINIMUMS[1]);
    }

    #[test]
    fn composer_width_and_other_panes_survive_toggling() {
        let mut sizes = PaneSizes::default();
        sizes.prepare(1400., true);
        let start = sizes.widths;
        sizes.resize(2, start, 55.);
        let open = sizes.widths;
        sizes.prepare(1400., false);
        sizes.prepare(1400., true);
        for (actual, expected) in sizes.widths.into_iter().zip(open) {
            close(actual, expected);
        }
    }

    #[test]
    fn viewport_resize_preserves_proportions_until_minima_are_reached() {
        let mut sizes = PaneSizes::default();
        sizes.prepare(1400., true);
        let start = sizes.widths;
        sizes.prepare(1800., true);
        close(sizes.widths[0] / sizes.widths[1], start[0] / start[1]);
        sizes.prepare(600., true);
        for (actual, expected) in sizes.widths.into_iter().zip(MINIMUMS) {
            close(actual, expected);
        }
    }
}
