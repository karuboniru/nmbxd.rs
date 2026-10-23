//! User preferences. Authentication values must never be included in diagnostics.
use anyhow::{Context as _, Result};
use gpui::{
    Context, Entity, EventEmitter, FontWeight, IntoElement, Render, Subscription, Window, div,
    prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Disableable, IndexPath,
    button::{Button, ButtonVariants},
    input::{Input, InputState},
    select::{SearchableVec, Select, SelectEvent, SelectState},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub const DEFAULT_FONT: &str = "Noto Sans CJK SC";
pub const DEFAULT_FONT_SIZE: f32 = 14.;
pub const DEFAULT_UI_FONT_SIZE: f32 = 16.;
const UI_FONT_SIZE_RANGE: (f32, f32) = (10., 24.);
const THREAD_FONT_SIZE_RANGE: (f32, f32) = (10., 32.);
const BODY_FONT_SIZE_RANGE: (f32, f32) = (10., 40.);

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub cookie: String,
    pub subscription_id: String,
    pub font_family: String,
    pub website_base: String,
    pub ui_font_size: f32,
    pub thread_font_size: f32,
    pub body_font_size: f32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            cookie: String::new(),
            subscription_id: uuid::Uuid::new_v4().to_string(),
            font_family: DEFAULT_FONT.into(),
            website_base: nmbxd::api::WEBSITE_BASE.into(),
            ui_font_size: DEFAULT_UI_FONT_SIZE,
            thread_font_size: DEFAULT_FONT_SIZE,
            body_font_size: DEFAULT_FONT_SIZE,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        std::env::var_os("XDG_CONFIG_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|value| !value.is_empty())
                    .map(|home| PathBuf::from(home).join(".config"))
            })
            .map(|base| base.join("nmbxd/settings.json"))
    }

    /// Persist a new subscription ID before the application can use it. Existing
    /// unreadable or malformed settings are reported without overwriting them.
    /// A saved empty cookie overrides the fallback for anonymous browsing.
    pub fn load_or_initialize(fallback_cookie: String) -> Result<Self> {
        let path = Self::path().context("无法确定配置目录")?;
        Self::load_or_initialize_from(&path, fallback_cookie)
    }

    fn load_or_initialize_from(path: &Path, fallback_cookie: String) -> Result<Self> {
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let settings = Self {
                    cookie: fallback_cookie,
                    ..Self::default()
                };
                settings.save_to(path)?;
                return Ok(settings);
            }
            Err(error) => return Err(error).context("无法读取设置文件"),
        };
        let stored: serde_json::Value =
            serde_json::from_slice(&bytes).context("无法解析设置文件")?;
        let needs_subscription_id = stored.get("subscription_id").is_none();
        let mut settings: Self = serde_json::from_value(stored).context("设置文件格式无效")?;
        settings.normalize_font_sizes();
        if needs_subscription_id {
            settings.save_to(path)?;
        }
        Ok(settings)
    }

    #[cfg(test)]
    fn load_from(path: &Path) -> Option<Self> {
        let mut settings: Self = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        settings.normalize_font_sizes();
        Some(settings)
    }

    fn normalize_font_sizes(&mut self) {
        self.ui_font_size =
            safe_font_size(self.ui_font_size, UI_FONT_SIZE_RANGE, DEFAULT_UI_FONT_SIZE);
        self.thread_font_size = safe_font_size(
            self.thread_font_size,
            THREAD_FONT_SIZE_RANGE,
            DEFAULT_FONT_SIZE,
        );
        self.body_font_size =
            safe_font_size(self.body_font_size, BODY_FONT_SIZE_RANGE, DEFAULT_FONT_SIZE);
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path().context("无法确定配置目录")?;
        self.save_to(&path)
    }

    fn save_to(&self, path: &Path) -> Result<()> {
        static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
        let subscription_id = nmbxd::api::validate_subscription_id(&self.subscription_id)?;
        let parent = path.parent().context("配置文件目录无效")?;
        fs::create_dir_all(parent).context("无法创建配置目录")?;
        // A new inode ensures an old, broadly readable file is never rewritten
        // with credentials before its permissions have been restricted.
        let mut settings = self.clone();
        settings.subscription_id = subscription_id.to_string();
        settings.normalize_font_sizes();
        let bytes = serde_json::to_vec_pretty(&settings).context("无法编码设置")?;
        let (temporary, mut file) = loop {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let temporary = parent.join(format!(".settings-{}-{id}.tmp", std::process::id()));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            match options.open(&temporary) {
                Ok(file) => break (temporary, file),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error).context("无法创建设置文件"),
            }
        };
        let result = (|| -> Result<()> {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o600))
                    .context("无法限制设置文件权限")?;
            }
            file.write_all(&bytes).context("无法写入设置")?;
            file.sync_all().context("无法同步设置")?;
            drop(file);
            fs::rename(&temporary, path).context("无法替换设置文件")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

fn safe_font_size(value: f32, (min, max): (f32, f32), default: f32) -> f32 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}

fn parse_font_size(value: &str, label: &str, (min, max): (f32, f32)) -> Result<f32, String> {
    value
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|size| size.is_finite() && (min..=max).contains(size))
        .ok_or_else(|| format!("{label}请输入 {min}–{max} 之间的数字。"))
}

pub enum SettingsEvent {
    Save(Settings),
    Close,
}

pub struct SettingsPanel {
    cookie: Entity<InputState>,
    subscription_id: Entity<InputState>,
    website_base: Entity<InputState>,
    ui_font_size: Entity<InputState>,
    thread_font_size: Entity<InputState>,
    body_font_size: Entity<InputState>,
    font: Entity<SelectState<SearchableVec<String>>>,
    preview_font: String,
    saving: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsPanel {}

impl SettingsPanel {
    pub fn new(mut settings: Settings, window: &mut Window, cx: &mut Context<Self>) -> Self {
        settings.normalize_font_sizes();
        let ui_font_size = cx
            .new(|cx| InputState::new(window, cx).default_value(settings.ui_font_size.to_string()));
        let thread_font_size = cx.new(|cx| {
            InputState::new(window, cx).default_value(settings.thread_font_size.to_string())
        });
        let body_font_size = cx.new(|cx| {
            InputState::new(window, cx).default_value(settings.body_font_size.to_string())
        });
        let cookie = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("userhash=… 或饼干原文")
                .default_value(settings.cookie)
        });
        let subscription_id = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("订阅 ID")
                .default_value(settings.subscription_id)
        });
        let website_base = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("https://www.nmbxd1.com/")
                .default_value(settings.website_base)
        });
        let mut fonts = cx.text_system().all_font_names();
        fonts.sort_by_key(|name| name.to_lowercase());
        fonts.dedup();
        let selected = fonts
            .iter()
            .position(|name| name == &settings.font_family)
            .or_else(|| fonts.iter().position(|name| name == DEFAULT_FONT))
            .or_else(|| (!fonts.is_empty()).then_some(0));
        let preview_font = selected
            .map(|index| fonts[index].clone())
            .unwrap_or(settings.font_family);
        let font = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(fonts),
                selected.map(|index| IndexPath::default().row(index)),
                window,
                cx,
            )
            .searchable(true)
        });
        let subscription = cx.subscribe(&font, |this, _, event, cx| {
            let SelectEvent::Confirm(Some(font)) = event else {
                return;
            };
            this.preview_font = font.clone();
            cx.notify();
        });
        let size_subscriptions = [&ui_font_size, &thread_font_size, &body_font_size]
            .into_iter()
            .map(|input| {
                cx.subscribe(input, |_, _, _: &gpui_component::input::InputEvent, cx| {
                    cx.notify()
                })
            });
        let mut subscriptions = vec![subscription];
        subscriptions.extend(size_subscriptions);
        Self {
            cookie,
            subscription_id,
            website_base,
            ui_font_size,
            thread_font_size,
            body_font_size,
            font,
            preview_font,
            saving: false,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn set_saving(&mut self, saving: bool, cx: &mut Context<Self>) {
        self.saving = saving;
        if saving {
            self.error = None;
        }
        cx.notify();
    }

    pub fn set_error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.error = Some(message.into());
        self.saving = false;
        cx.notify();
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .id("settings-panel")
            .flex()
            .flex_col()
            .gap_4()
            .w(px(560.))
            .max_w_full()
            .max_h_full()
            .overflow_y_scroll()
            .p_6()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("设置"),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_sm().child("饼干"))
                    .child(Input::new(&self.cookie).mask_toggle().disabled(self.saving))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("支持 userhash=… 或原文；留空后以游客身份浏览。"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_sm().child("订阅 ID"))
                    .child(Input::new(&self.subscription_id).disabled(self.saving))
                    .child(div().text_xs().text_color(theme.muted_foreground).child(
                        "已自动生成随机 ID；同 ID 可跨设备共享。知道此 ID 的人可以查看和修改订阅，请妥善保存。",
                    )),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_sm().child("网页地址（发串 / 回复）"))
                    .child(Input::new(&self.website_base).disabled(self.saving))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("填写 HTTPS 网站域名；浏览使用的 JSON API 地址保持不变。"),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(div().text_sm().child("界面字体"))
                    .child(
                        Select::new(&self.font)
                            .w_full()
                            .search_placeholder("搜索系统字体…")
                            .placeholder("选择系统字体")
                            .disabled(self.saving),
                    ),
            )
            .child(
                div().flex().gap_3().children(
                    [
                        ("UI字号", "10–24", &self.ui_font_size),
                        ("串列表字号", "10–32", &self.thread_font_size),
                        ("正文字号", "10–40", &self.body_font_size),
                    ]
                    .into_iter()
                    .map(|(label, range, input)| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(div().text_sm().child(format!("{label}（{range}）")))
                            .child(Input::new(input).disabled(self.saving))
                    }),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_4()
                    .rounded_md()
                    .bg(theme.muted)
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("字体预览"),
                    )
                    .child(
                        div()
                            .font_family(self.preview_font.clone())
                            .text_size(px(parse_font_size(
                                self.body_font_size.read(cx).value().as_ref(),
                                "正文字号",
                                BODY_FONT_SIZE_RANGE,
                            )
                            .unwrap_or(DEFAULT_FONT_SIZE)))
                            .child(
                                "岛上日常，自在交流。Aa Bb Cc 0123456789
😀 👍🏽 ❤️ 👩‍💻 👨‍👩‍👧‍👦 🇨🇳 1️⃣",
                            ),
                    ),
            )
            .when_some(self.error.as_ref(), |this, message| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(theme.danger)
                        .child(message.clone()),
                )
            })
            .child(
                div()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("settings-close")
                            .label("关闭")
                            .disabled(self.saving)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
                    )
                    .child(
                        Button::new("settings-save")
                            .primary()
                            .label(if self.saving {
                                "保存中…"
                            } else {
                                "保存"
                            })
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.saving {
                                    return;
                                }
                                let subscription_id = match nmbxd::api::validate_subscription_id(
                                    this.subscription_id.read(cx).value().as_ref(),
                                ) {
                                    Ok(id) => id.to_string(),
                                    Err(error) => {
                                        this.set_error(error.to_string(), cx);
                                        return;
                                    }
                                };
                                let sizes = (|| {
                                    Ok::<_, String>((
                                        parse_font_size(
                                            this.ui_font_size.read(cx).value().as_ref(),
                                            "UI字号",
                                            UI_FONT_SIZE_RANGE,
                                        )?,
                                        parse_font_size(
                                            this.thread_font_size.read(cx).value().as_ref(),
                                            "串列表字号",
                                            THREAD_FONT_SIZE_RANGE,
                                        )?,
                                        parse_font_size(
                                            this.body_font_size.read(cx).value().as_ref(),
                                            "正文字号",
                                            BODY_FONT_SIZE_RANGE,
                                        )?,
                                    ))
                                })();
                                let (ui_font_size, thread_font_size, body_font_size) = match sizes {
                                    Ok(sizes) => sizes,
                                    Err(message) => {
                                        this.set_error(message, cx);
                                        return;
                                    }
                                };
                                let settings = Settings {
                                    cookie: this.cookie.read(cx).value().to_string(),
                                    subscription_id,
                                    font_family: this.preview_font.clone(),
                                    website_base: this.website_base.read(cx).value().to_string(),
                                    ui_font_size,
                                    thread_font_size,
                                    body_font_size,
                                };
                                this.set_saving(true, cx);
                                cx.emit(SettingsEvent::Save(settings));
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn default_subscription_ids_are_random_uuid_v4_values() {
        let first = Settings::default().subscription_id;
        let second = Settings::default().subscription_id;
        assert_eq!(uuid::Uuid::parse_str(&first).unwrap().get_version_num(), 4);
        assert_eq!(uuid::Uuid::parse_str(&second).unwrap().get_version_num(), 4);
        assert_ne!(first, second);
    }

    #[test]
    fn startup_initializes_and_migrates_subscription_ids_once() {
        let directory = std::env::temp_dir().join(format!(
            "nmbxd-settings-startup-test-{}",
            uuid::Uuid::new_v4()
        ));
        let path = directory.join("settings.json");
        let initial = Settings::load_or_initialize_from(&path, "fallback-only".into()).unwrap();
        assert_eq!(initial.cookie, "fallback-only");
        let loaded = Settings::load_or_initialize_from(&path, "ignored".into()).unwrap();
        assert_eq!(loaded.subscription_id, initial.subscription_id);
        assert_eq!(loaded.cookie, initial.cookie);

        fs::write(
            &path,
            br#"{"cookie":"saved-cookie","font_family":"Legacy Font"}"#,
        )
        .unwrap();
        let migrated = Settings::load_or_initialize_from(&path, "ignored".into()).unwrap();
        assert_eq!(migrated.cookie, "saved-cookie");
        assert_eq!(migrated.font_family, "Legacy Font");
        assert_eq!(
            Settings::load_from(&path).unwrap().subscription_id,
            migrated.subscription_id
        );
        let saved_bytes = fs::read(&path).unwrap();
        let reloaded = Settings::load_or_initialize_from(&path, "ignored".into()).unwrap();
        assert_eq!(reloaded.subscription_id, migrated.subscription_id);
        assert_eq!(fs::read(&path).unwrap(), saved_bytes);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn startup_preserves_malformed_settings() {
        let directory = std::env::temp_dir().join(format!(
            "nmbxd-settings-invalid-test-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        for bytes in [
            b"{invalid".as_slice(),
            br#"{"subscription_id":null}"#.as_slice(),
        ] {
            fs::write(&path, bytes).unwrap();
            assert!(Settings::load_or_initialize_from(&path, String::new()).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn legacy_settings_default_website_and_custom_website_persists() {
        let directory = std::env::temp_dir().join(format!(
            "nmbxd-settings-migration-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        fs::write(&path, br#"{"cookie":"","font_family":"Legacy Font"}"#).unwrap();
        let mut settings = Settings::load_from(&path).unwrap();
        assert_eq!(settings.website_base, "https://www.nmbxd1.com/");
        assert_eq!(settings.font_family, "Legacy Font");
        assert_eq!(settings.ui_font_size, DEFAULT_UI_FONT_SIZE);
        assert_eq!(settings.thread_font_size, DEFAULT_FONT_SIZE);
        assert_eq!(settings.body_font_size, DEFAULT_FONT_SIZE);
        let subscription_id = settings.subscription_id.clone();
        assert_eq!(
            uuid::Uuid::parse_str(&subscription_id)
                .unwrap()
                .get_version_num(),
            4
        );
        settings.website_base = "https://island.example/".into();
        settings.ui_font_size = 18.;
        settings.thread_font_size = 17.5;
        settings.body_font_size = 20.;
        settings.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path).unwrap();
        assert_eq!(loaded.website_base, "https://island.example/");
        assert_eq!(loaded.font_family, "Legacy Font");
        assert_eq!(loaded.ui_font_size, 18.);
        assert_eq!(loaded.thread_font_size, 17.5);
        assert_eq!(loaded.body_font_size, 20.);
        assert_eq!(loaded.subscription_id, subscription_id);
        assert!(loaded.cookie.is_empty());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn edited_subscription_id_is_validated_and_persists() {
        let directory = std::env::temp_dir().join(format!(
            "nmbxd-settings-subscription-test-{}",
            uuid::Uuid::new_v4()
        ));
        let path = directory.join("settings.json");
        let mut settings = Settings {
            subscription_id: "   ".into(),
            ..Settings::default()
        };
        assert!(settings.save_to(&path).is_err());
        assert!(!directory.exists());
        settings.subscription_id = "岛".repeat(nmbxd::api::MAX_SUBSCRIPTION_ID_CHARS + 1);
        assert!(settings.save_to(&path).is_err());
        assert!(!directory.exists());
        settings.subscription_id = "  我的共享订阅 ID  ".into();
        settings.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path).unwrap();
        assert_eq!(loaded.subscription_id, "我的共享订阅 ID");
        loaded.save_to(&path).unwrap();
        assert_eq!(
            Settings::load_from(&path).unwrap().subscription_id,
            loaded.subscription_id
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn font_size_inputs_require_finite_numbers_within_each_range() {
        assert_eq!(
            parse_font_size(" 17.5 ", "UI字号", UI_FONT_SIZE_RANGE),
            Ok(17.5)
        );
        for value in ["", "abc", "NaN", "inf", "-inf", "9.9", "24.1"] {
            assert!(parse_font_size(value, "UI字号", UI_FONT_SIZE_RANGE).is_err());
        }
        assert!(parse_font_size("32", "串列表字号", THREAD_FONT_SIZE_RANGE).is_ok());
        assert!(parse_font_size("40", "正文字号", BODY_FONT_SIZE_RANGE).is_ok());
    }

    #[test]
    fn persisted_font_sizes_are_clamped_and_nonfinite_values_save_as_defaults() {
        let directory = std::env::temp_dir().join(format!(
            "nmbxd-settings-size-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.json");
        fs::write(
            &path,
            br#"{"ui_font_size":100,"thread_font_size":-1,"body_font_size":100}"#,
        )
        .unwrap();
        let mut settings = Settings::load_from(&path).unwrap();
        assert_eq!(settings.ui_font_size, 24.);
        assert_eq!(settings.thread_font_size, 10.);
        assert_eq!(settings.body_font_size, 40.);
        settings.ui_font_size = f32::NAN;
        settings.thread_font_size = f32::INFINITY;
        settings.body_font_size = f32::NEG_INFINITY;
        settings.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path).unwrap();
        assert_eq!(loaded.ui_font_size, DEFAULT_UI_FONT_SIZE);
        assert_eq!(loaded.thread_font_size, DEFAULT_FONT_SIZE);
        assert_eq!(loaded.body_font_size, DEFAULT_FONT_SIZE);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn saved_empty_cookie_remains_distinct_from_missing_settings() {
        let directory = std::env::temp_dir().join(format!(
            "nmbxd-settings-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("settings.json");
        assert!(Settings::load_from(&path).is_none());
        let mut settings = Settings {
            cookie: "userhash=test-only-value".into(),
            ..Settings::default()
        };
        settings.save_to(&path).unwrap();
        settings.cookie.clear();
        settings.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path).unwrap();
        assert!(loaded.cookie.is_empty());
        assert_eq!(loaded.font_family, DEFAULT_FONT);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}
