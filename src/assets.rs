//! Small embedded UI glyphs; release builds do not depend on loose asset files.
use gpui::{AssetSource, SharedString};
use std::borrow::Cow;

pub struct Assets;
const ICONS: &[(&str, &str)] = &[
    ("icons/chevron-down.svg", "<path d='m6 9 6 6 6-6'/>"),
    ("icons/check.svg", "<path d='m5 12 4 4L19 6'/>"),
    ("icons/close.svg", "<path d='m6 6 12 12M6 18 18 6'/>"),
    (
        "icons/circle-x.svg",
        "<circle cx='12' cy='12' r='9'/><path d='m9 9 6 6m-6 0 6-6'/>",
    ),
    (
        "icons/search.svg",
        "<circle cx='10' cy='10' r='6'/><path d='m15 15 6 6'/>",
    ),
    (
        "icons/eye.svg",
        "<path d='M2 12Q12-1 22 12Q12 25 2 12Z'/><circle cx='12' cy='12' r='3'/>",
    ),
    (
        "icons/eye-off.svg",
        "<path d='M2 12Q12-1 22 12Q12 25 2 12ZM3 3l18 18'/><circle cx='12' cy='12' r='3'/>",
    ),
    (
        "icons/inbox.svg",
        "<path d='M4 4h16v16H4ZM4 13h5l1 3h4l1-3h5'/>",
    ),
    (
        "icons/loader-circle.svg",
        "<path d='M21 12a9 9 0 1 1-9-9'/>",
    ),
];
impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS.iter().find(|(name, _)| *name == path).map(|(_, body)| {
            Cow::Owned(format!("<svg xmlns='http://www.w3.org/2000/svg' width='24' height='24' viewBox='0 0 24 24' fill='none' stroke='black' stroke-width='1.8' stroke-linecap='round' stroke-linejoin='round'>{body}</svg>").into_bytes())
        }))
    }
    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| (*name).into())
            .collect())
    }
}
