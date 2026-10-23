//! Appearance preferences contain no authentication or draft data.
use gpui::{App, Window};
use gpui_component::{Theme, ThemeMode};
use serde::{Deserialize, Serialize};
use std::path::Path;

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
    pub fn load() -> Self {
        crate::config_paths::paths("appearance.json")
            .map(|(path, legacy)| Self::load_from_paths(&path, &legacy))
            .unwrap_or_default()
    }
    fn load_from_paths(path: &Path, legacy: &Path) -> Self {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => Some(bytes),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::read(legacy).ok()
            }
            Err(_) => None,
        };
        bytes
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }
    pub fn save(self) -> anyhow::Result<()> {
        let (path, _) = crate::config_paths::paths("appearance.json")
            .ok_or_else(|| anyhow::anyhow!("无法确定配置目录"))?;
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
    fn rebrand_falls_back_to_legacy_only_when_current_file_is_missing() {
        let directory = std::env::temp_dir().join(format!(
            "rdnmb-appearance-rebrand-test-{}",
            uuid::Uuid::new_v4()
        ));
        let legacy = directory.join("nmbxd/appearance.json");
        let current = directory.join("rdnmb/appearance.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        assert_eq!(
            Appearance::load_from_paths(&current, &legacy),
            Appearance::System
        );
        std::fs::write(&legacy, br#""dark""#).unwrap();
        assert_eq!(
            Appearance::load_from_paths(&current, &legacy),
            Appearance::Dark
        );
        std::fs::create_dir_all(current.parent().unwrap()).unwrap();
        std::fs::write(&current, br#""light""#).unwrap();
        assert_eq!(
            Appearance::load_from_paths(&current, &legacy),
            Appearance::Light
        );
        std::fs::write(&current, b"{invalid").unwrap();
        assert_eq!(
            Appearance::load_from_paths(&current, &legacy),
            Appearance::System
        );
        assert_eq!(std::fs::read(&legacy).unwrap(), br#""dark""#);
        std::fs::remove_dir_all(directory).unwrap();
    }

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
