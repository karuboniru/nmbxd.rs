# Local patch to gpui-component 0.5.1

Source: published crates.io package gpui-component 0.5.1 (Apache-2.0).
Only `src/window_border.rs` is modified:

- Use current `Window::viewport_size()` for resize hit-testing rather than
  `WindowBounds::get_bounds()`, whose maximized/fullscreen bounds are restore bounds.
- Disable edge resizing while maximized/fullscreen.
- Respect individual tiled-edge paddings in both cursor and mouse handling.
- Add regression tests for floating and tiled window edges.

This prevents right-side image-viewer controls being intercepted as window resize
operations after maximizing. Re-evaluate/remove when upgrading gpui-component.

The parent workspace owns dependency resolution; the upstream package-local
resolver declaration was omitted. tests/fixtures/layout.json is retained because
upstream library tests include it even when only border regressions are run.
