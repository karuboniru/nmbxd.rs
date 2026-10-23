//! Appearance preferences contain no authentication or draft data.
use gpui::{App, Window};
use gpui_component::{Theme, ThemeMode};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub fn next(self) -> Self {
        match self {
            Self::System => Self::Light,
            Self::Light => Self::Dark,
            Self::Dark => Self::System,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::System => "外观：跟随系统",
            Self::Light => "外观：日间",
            Self::Dark => "外观：夜间",
        }
    }
    fn path() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                #[cfg(target_os = "windows")]
                {
                    std::env::var_os("APPDATA")
                        .filter(|value| !value.is_empty())
                        .map(PathBuf::from)
                        .filter(|path| path.is_absolute())
                }
                #[cfg(not(target_os = "windows"))]
                None
            })
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .map(|base| base.join("nmbxd/appearance.json"))
    }
    pub fn load() -> Self {
        Self::path()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    pub fn save(self) -> anyhow::Result<()> {
        let path = Self::path().ok_or_else(|| anyhow::anyhow!("无法确定配置目录"))?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        std::fs::write(&temporary, serde_json::to_vec(&self)?)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }
    pub fn apply(self, window: &mut Window, cx: &mut App) {
        let mode = match self {
            Self::System => ThemeMode::from(window.appearance()),
            Self::Light => ThemeMode::Light,
            Self::Dark => ThemeMode::Dark,
        };
        Theme::change(mode, Some(window), cx);
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub background: u32,
    pub foreground: u32,
    pub muted: u32,
    pub accent: u32,
    pub surface: u32,
    pub sidebar: u32,
    pub border: u32,
    pub selected: u32,
    pub hover: u32,
}

impl Palette {
    pub fn current(cx: &App) -> Self {
        if Theme::global(cx).is_dark() {
            Self {
                background: 0x1b2026,
                foreground: 0xe1e7ed,
                muted: 0xa0acb8,
                accent: 0x8dd2bf,
                surface: 0x252c34,
                sidebar: 0x20262d,
                border: 0x39434d,
                selected: 0x28483f,
                hover: 0x314039,
            }
        } else {
            Self {
                background: 0xf6f7f9,
                foreground: 0x273442,
                muted: 0x647382,
                accent: 0x287568,
                surface: 0xffffff,
                sidebar: 0xf0f3f4,
                border: 0xe1e6ec,
                selected: 0xe2efea,
                hover: 0xe7f1ee,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preference_round_trips_and_cycles_to_system() {
        for mode in [Appearance::System, Appearance::Light, Appearance::Dark] {
            let json = serde_json::to_string(&mode).unwrap();
            assert_eq!(serde_json::from_str::<Appearance>(&json).unwrap(), mode);
        }
        assert_eq!(Appearance::System.next().next().next(), Appearance::System);
        assert!(serde_json::from_str::<Appearance>("\"unknown\"").is_err());
    }
}
