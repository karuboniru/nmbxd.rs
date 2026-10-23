//! Offline emoji fallback for the pinned GPUI Linux text renderer.

use gpui::App;

/// Register before creating any views or shaping text. Ordinary text keeps the
/// user's selected font; cosmic-text chooses this font for missing emoji glyphs.
pub fn init(cx: &mut App) -> anyhow::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use std::borrow::Cow;

        let text_system = cx.text_system();

        // GPUI 0.2.2 / Swash cannot rasterize COLRv1 (the default Noto Color
        // Emoji on some Linux distributions). Simply adding our CBDT font is
        // insufficient: cosmic-text would prefer the earlier system face.
        //
        // GPUI's Linux load_family removes faces without an 'm' glyph when
        // resolving a primary family. Resolve the existing emoji-only family
        // once, before registering the compatible font, to remove those faces.
        // The resolved primary font falls back to GPUI's normal UI font stack;
        // the user's chosen text/CJK families are never removed or changed.
        // This workaround depends on the exact pinned GPUI implementation and
        // should be revisited when GPUI gains COLRv1 or explicit Linux fallbacks.
        if text_system
            .all_font_names()
            .iter()
            .any(|name| name == "Noto Color Emoji")
        {
            text_system.resolve_font(&gpui::font("Noto Color Emoji"));
        }

        text_system.add_fonts(vec![Cow::Borrowed(include_bytes!(
            "../assets/fonts/NotoColorEmoji.ttf"
        ))])?;
    }

    #[cfg(not(target_os = "linux"))]
    let _ = cx;

    Ok(())
}
