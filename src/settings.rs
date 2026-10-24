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
    collections::BTreeMap,
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
    /// Runtime value, accepted only when reading the old settings schema.
    #[serde(skip_serializing)]
    pub cookie: String,
    #[serde(skip)]
    pub cookies: BTreeMap<String, String>,
    /// Explicit removals with the original values used to detect file changes.
    #[serde(skip)]
    pub deleted_cookies: BTreeMap<String, String>,
    pub auth_cookie: Option<String>,
    pub subscription_id: String,
    pub font_family: String,
    pub website_base: String,
    pub ui_font_size: f32,
    pub thread_font_size: f32,
    pub body_font_size: f32,
    pub pane_layout: Option<crate::panes::PanePreferences>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            cookie: String::new(),
            cookies: BTreeMap::new(),
            deleted_cookies: BTreeMap::new(),
            auth_cookie: None,
            subscription_id: uuid::Uuid::new_v4().to_string(),
            font_family: DEFAULT_FONT.into(),
            website_base: rdnmb::api::WEBSITE_BASE.into(),
            ui_font_size: DEFAULT_UI_FONT_SIZE,
            thread_font_size: DEFAULT_FONT_SIZE,
            body_font_size: DEFAULT_FONT_SIZE,
            pane_layout: None,
        }
    }
}

impl Settings {
    fn path() -> Option<PathBuf> {
        crate::config_paths::paths("settings.json").map(|(current, _)| current)
    }

    /// Persist a new subscription ID before the application can use it. Existing
    /// unreadable or malformed settings are reported without overwriting them.
    /// A saved guest selection overrides the environment fallback.
    pub fn load_or_initialize(fallback_cookie: String) -> Result<Self> {
        let (path, legacy) =
            crate::config_paths::paths("settings.json").context("无法确定配置目录")?;
        Self::load_or_initialize_paths(&path, Some(&legacy), fallback_cookie)
    }

    #[cfg(test)]
    fn load_or_initialize_from(path: &Path, fallback_cookie: String) -> Result<Self> {
        Self::load_or_initialize_paths(path, None, fallback_cookie)
    }

    fn load_or_initialize_paths(
        path: &Path,
        legacy: Option<&Path>,
        fallback_cookie: String,
    ) -> Result<Self> {
        let (bytes, migrating) = match fs::read(path) {
            Ok(bytes) => (Some(bytes), false),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let bytes = match legacy.map(fs::read) {
                    Some(Ok(bytes)) => Some(bytes),
                    Some(Err(error)) if error.kind() != std::io::ErrorKind::NotFound => {
                        return Err(error).context("无法读取旧设置文件");
                    }
                    _ => None,
                };
                (bytes, true)
            }
            Err(error) => return Err(error).context("无法读取设置文件"),
        };
        let stored = bytes
            .as_ref()
            .map(|bytes| {
                serde_json::from_slice::<serde_json::Value>(bytes).context("无法解析设置文件")
            })
            .transpose()?;
        let needs_subscription_id = stored
            .as_ref()
            .is_none_or(|stored| stored.get("subscription_id").is_none());
        let has_auth_selection = stored
            .as_ref()
            .is_some_and(|stored| stored.get("auth_cookie").is_some());
        let has_legacy_cookie = stored
            .as_ref()
            .is_some_and(|stored| stored.get("cookie").is_some());
        let mut settings: Self = match stored {
            Some(stored) => serde_json::from_value(stored).context("设置文件格式无效")?,
            None => Self::default(),
        };
        settings.normalize_font_sizes();
        // Validate the settings before any credential migration is written.
        rdnmb::api::validate_subscription_id(&settings.subscription_id)?;
        let directory = path.parent().context("配置文件目录无效")?.join("cookie");
        settings.cookies = crate::cookies::load_from(&directory)?;
        if !has_auth_selection || has_legacy_cookie {
            let old_cookie = if bytes.is_none() {
                fallback_cookie
            } else {
                settings.cookie.clone()
            };
            if !old_cookie.trim().is_empty() {
                crate::cookies::validate_cookie(&old_cookie)?;
                let name = if settings
                    .cookies
                    .get("default")
                    .is_none_or(|value| value == &old_cookie)
                {
                    "default".to_owned()
                } else if let Some((name, _)) = settings
                    .cookies
                    .iter()
                    .find(|(_, value)| *value == &old_cookie)
                {
                    name.clone()
                } else {
                    // Preserve both an existing default and the old active value.
                    (1u64..)
                        .map(|index| format!("legacy-{index}"))
                        .find(|name| !settings.cookies.contains_key(name))
                        .unwrap()
                };
                crate::cookies::store_new_in(&directory, &name, &old_cookie)?;
                settings.cookies.insert(name.clone(), old_cookie);
                if !has_auth_selection {
                    settings.auth_cookie = Some(name);
                }
            } else if !has_auth_selection
                && bytes.is_none()
                && settings.cookies.contains_key("default")
            {
                settings.auth_cookie = Some("default".into());
            }
        }
        // A removed credential must not make the settings UI inaccessible.
        // Missing selections fall back to guest; all other credentials remain.
        if settings
            .auth_cookie
            .as_ref()
            .is_some_and(|name| !settings.cookies.contains_key(name))
        {
            if let Some(name) = &settings.auth_cookie {
                crate::cookies::validate_name(name)?;
            }
            settings.auth_cookie = None;
        }
        settings.resolve_cookie()?;
        if migrating || needs_subscription_id || !has_auth_selection || has_legacy_cookie {
            settings.save_to(path)?;
        }
        Ok(settings)
    }

    #[cfg(test)]
    fn load_from(path: &Path) -> Option<Self> {
        let mut settings: Self = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
        settings.normalize_font_sizes();
        settings.cookies = crate::cookies::load_from(&path.parent()?.join("cookie")).ok()?;
        settings.resolve_cookie().ok()?;
        Some(settings)
    }

    fn resolve_cookie(&mut self) -> Result<()> {
        self.cookie = match &self.auth_cookie {
            Some(name) => {
                crate::cookies::validate_name(name)?;
                self.cookies
                    .get(name)
                    .context("认证饼干文件不存在，请恢复文件或在设置中选择游客")?
                    .clone()
            }
            None => String::new(),
        };
        Ok(())
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
        let subscription_id = rdnmb::api::validate_subscription_id(&self.subscription_id)?;
        let parent = path.parent().context("配置文件目录无效")?;
        // A new inode ensures an old, broadly readable file is never rewritten
        // with credentials before its permissions have been restricted.
        let mut settings = self.clone();
        settings.subscription_id = subscription_id.to_string();
        settings.normalize_font_sizes();
        settings.resolve_cookie()?;
        for (name, cookie) in &settings.cookies {
            crate::cookies::validate_name(name)?;
            crate::cookies::validate_cookie(cookie)?;
        }
        let cookie_directory = parent.join("cookie");
        for (name, expected) in &settings.deleted_cookies {
            anyhow::ensure!(
                !settings.cookies.contains_key(name),
                "待删除的饼干仍在保存列表中，请重新打开设置后重试"
            );
            crate::cookies::check_removal_in(&cookie_directory, name, expected)?;
        }
        fs::create_dir_all(parent).context("无法创建配置目录")?;
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
        let mut removed = Vec::new();
        let mut imported = Vec::new();
        let result = (|| -> Result<()> {
            for (name, cookie) in &settings.cookies {
                if crate::cookies::store_new_tracked_in(&cookie_directory, name, cookie)? {
                    imported.push((name, cookie));
                }
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(0o600))
                    .context("无法限制设置文件权限")?;
            }
            file.write_all(&bytes).context("无法写入设置")?;
            file.sync_all().context("无法同步设置")?;
            drop(file);
            for (name, expected) in &settings.deleted_cookies {
                if crate::cookies::remove_if_matches_in(&cookie_directory, name, expected)? {
                    removed.push((name, expected));
                }
            }
            fs::rename(&temporary, path).context("无法替换设置文件")?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
            // Keep the previous settings usable when publishing the new file
            // fails. create_new also protects files changed during rollback.
            let mut restore_failed = false;
            for (name, cookie) in removed {
                if crate::cookies::store_new_in(&cookie_directory, name, cookie).is_err() {
                    restore_failed = true;
                }
            }
            for (name, cookie) in imported {
                if crate::cookies::remove_if_matches_in(&cookie_directory, name, cookie).is_err() {
                    restore_failed = true;
                }
            }
            if restore_failed {
                return result
                    .context("保存失败，且部分饼干文件无法恢复原状；请检查饼干目录后重试");
            }
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

fn import_cookie(
    cookies: &mut BTreeMap<String, String>,
    deleted: &mut BTreeMap<String, String>,
    removed: &mut BTreeMap<String, String>,
    name: String,
    cookie: String,
) -> Result<()> {
    if cookies
        .get(&name)
        .or_else(|| removed.get(&name))
        .or_else(|| deleted.get(&name))
        .is_some_and(|old| old != &cookie)
    {
        anyhow::bail!("同名饼干已存在；请先修改导入来源的名称，现有饼干未被覆盖。");
    }
    deleted.remove(&name);
    removed.remove(&name);
    cookies.insert(name, cookie);
    Ok(())
}

fn remove_cookie(
    cookies: &mut BTreeMap<String, String>,
    saved: &BTreeMap<String, String>,
    deleted: &mut BTreeMap<String, String>,
    removed: &mut BTreeMap<String, String>,
    auth_cookie: &mut Option<String>,
    name: &str,
) {
    if let Some(cookie) = cookies.remove(name) {
        removed.insert(name.to_owned(), cookie);
        if let Some(original) = saved.get(name) {
            deleted.insert(name.to_owned(), original.clone());
        }
        if auth_cookie.as_deref() == Some(name) {
            *auth_cookie = None;
        }
    }
}

fn restore_cookie(
    cookies: &mut BTreeMap<String, String>,
    deleted: &mut BTreeMap<String, String>,
    removed: &mut BTreeMap<String, String>,
    name: &str,
) {
    if let Some(cookie) = removed.remove(name) {
        deleted.remove(name);
        cookies.insert(name.to_owned(), cookie);
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
    cookie_import: Entity<InputState>,
    cookies: BTreeMap<String, String>,
    saved_cookies: BTreeMap<String, String>,
    deleted_cookies: BTreeMap<String, String>,
    /// Includes unsaved imports, which must never become disk removals.
    removed_cookies: BTreeMap<String, String>,
    auth_cookie: Option<String>,
    import_status: Option<String>,
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
        let cookie_import = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("粘贴二维码图片链接 https://publicapi.ovear.info/qrcode.php?…")
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
            cookie_import,
            saved_cookies: settings
                .cookies
                .iter()
                .chain(settings.deleted_cookies.iter())
                .map(|(name, cookie)| (name.clone(), cookie.clone()))
                .collect(),
            removed_cookies: settings.deleted_cookies.clone(),
            cookies: settings.cookies,
            deleted_cookies: settings.deleted_cookies,
            auth_cookie: settings.auth_cookie,
            import_status: None,
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
                    .child(Input::new(&self.cookie_import).mask_toggle().disabled(self.saving))
                    .child(
                        Button::new("settings-import-cookie")
                            .label("导入二维码链接")
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, window, cx| {
                                let imported = crate::cookies::parse_import_url(
                                    this.cookie_import.read(cx).value().as_ref(),
                                );
                                match imported {
                                    Ok((name, cookie)) => {
                                        if let Err(error) = import_cookie(&mut this.cookies, &mut this.deleted_cookies, &mut this.removed_cookies, name.clone(), cookie) {
                                            this.set_error(error.to_string(), cx);
                                            return;
                                        }
                                        this.import_status = Some(format!("已添加“{name}”，保存后生效。可继续导入其他饼干。"));
                                        this.error = None;
                                        this.cookie_import.update(cx, |input, cx| input.set_value("", window, cx));
                                        cx.notify();
                                    }
                                    Err(error) => this.set_error(error.to_string(), cx),
                                }
                            })),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("在网页版饼干二维码上右键选择“复制图像链接”，粘贴后导入，无需 F12。可逐个导入多个饼干；链接仅在本机解码，饼干原文保存为配置目录中的 cookie/<名称>.cookie。删除当前认证饼干会切换为游客；恢复后需重新选择认证饼干。"),
                    )
                    .child(div().text_xs().text_color(theme.muted_foreground)
                        .child("点击保存后生效；关闭设置而不保存将撤回所有改动。"))
                    .when_some(self.import_status.as_ref(), |this, message| {
                        this.child(div().text_xs().child(message.clone()))
                    })
                    .child(div().text_sm().child("浏览认证 / 发帖默认饼干"))
                    .child(
                        Button::new("settings-auth-guest")
                            .label(if self.auth_cookie.is_none() { "✓ 游客（不使用饼干）" } else { "游客（不使用饼干）" })
                            .when(self.auth_cookie.is_none(), |button| button.primary())
                            .disabled(self.saving)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.auth_cookie = None;
                                cx.notify();
                            })),
                    )
                    .children(self.cookies.keys().chain(self.removed_cookies.keys())
                        .map(|name| (name.clone(), ())).collect::<BTreeMap<_, _>>()
                        .into_keys().enumerate().map(|(index, name)| {
                        let removed = self.removed_cookies.contains_key(&name);
                        let selected = self.auth_cookie.as_ref() == Some(&name) && !removed;
                        let action_name = name.clone();
                        div().flex().w_full().min_w_0().gap_2().rounded_md()
                            .border_1().border_color(theme.transparent)
                            .when(removed, |row| row.border_dashed().border_color(theme.border).bg(theme.muted))
                            .child(Button::new(("settings-auth-cookie", index))
                                .flex_1().min_w_0().overflow_hidden()
                                .child(div().relative().min_w_0().px_5()
                                    .child(div().truncate().child(name.clone()))
                                    .when(selected, |label| label.child(div().absolute().left_0().top_0().bottom_0().flex().items_center().child("✓"))))
                                .when(selected, |button| button.primary())
                                .when(removed, |button| button.border_dashed().border_color(theme.border)
                                    .bg(theme.muted).text_color(theme.muted_foreground))
                                .disabled(self.saving || removed)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.auth_cookie = Some(name.clone());
                                    cx.notify();
                                })))
                            .child(Button::new(("settings-cookie-action", index))
                                .w(px(64.)).flex_shrink_0()
                                .label(if removed { "恢复" } else { "删除" })
                                .disabled(self.saving)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if removed {
                                        restore_cookie(&mut this.cookies, &mut this.deleted_cookies, &mut this.removed_cookies, &action_name);
                                        this.import_status = Some(format!("已恢复“{action_name}”。如需用于认证，请重新选择。"));
                                    } else {
                                        remove_cookie(&mut this.cookies, &this.saved_cookies, &mut this.deleted_cookies, &mut this.removed_cookies, &mut this.auth_cookie, &action_name);
                                        this.import_status = Some(format!("“{action_name}”待删除；可点击恢复。"));
                                    }
                                    this.error = None;
                                    cx.notify();
                                })))
                    })),
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
                                let subscription_id = match rdnmb::api::validate_subscription_id(
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
                                    cookie: this.auth_cookie.as_ref().and_then(|name| this.cookies.get(name)).cloned().unwrap_or_default(),
                                    cookies: this.cookies.clone(),
                                    deleted_cookies: this.deleted_cookies.clone(),
                                    auth_cookie: this.auth_cookie.clone(),
                                    subscription_id,
                                    font_family: this.preview_font.clone(),
                                    website_base: this.website_base.read(cx).value().to_string(),
                                    ui_font_size,
                                    thread_font_size,
                                    body_font_size,
                                    pane_layout: None,
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
    fn pane_layout_round_trips_in_settings_and_old_files_use_defaults() {
        let directory = std::env::temp_dir().join(format!("rdnmb-layout-{}", uuid::Uuid::new_v4()));
        let path = directory.join("settings.json");
        let mut settings = Settings::default();
        let layout = serde_json::json!({
            "widths": [210., 250., 600., 320.], "container": 1072., "composing": false
        });
        settings.pane_layout = Some(serde_json::from_value(layout.clone()).unwrap());
        settings.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path).unwrap();
        assert_eq!(serde_json::to_value(loaded.pane_layout).unwrap(), layout);
        let mut json = serde_json::to_value(settings).unwrap();
        json.as_object_mut().unwrap().remove("pane_layout");
        assert!(
            serde_json::from_value::<Settings>(json)
                .unwrap()
                .pane_layout
                .is_none()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    fn remove_saved_cookie(settings: &mut Settings, name: &str) {
        let saved = settings.cookies.clone();
        remove_cookie(
            &mut settings.cookies,
            &saved,
            &mut settings.deleted_cookies,
            &mut BTreeMap::new(),
            &mut settings.auth_cookie,
            name,
        );
    }

    #[test]
    fn deleting_selected_cookie_is_staged_then_persists_as_guest() {
        let directory = std::env::temp_dir().join(format!("rdnmb-delete-{}", uuid::Uuid::new_v4()));
        let path = directory.join("settings.json");
        let original = Settings::load_or_initialize_from(&path, "selected-secret".into()).unwrap();
        let mut edited = original.clone();
        remove_saved_cookie(&mut edited, "default");
        assert!(edited.auth_cookie.is_none());
        assert_eq!(
            Settings::load_from(&path).unwrap().cookie,
            "selected-secret"
        );
        assert_eq!(original.auth_cookie.as_deref(), Some("default"));
        edited.save_to(&path).unwrap();
        // Retrying with the same pending removals is harmless.
        edited.save_to(&path).unwrap();
        let loaded = Settings::load_or_initialize_from(&path, "ignored-fallback".into()).unwrap();
        assert!(loaded.cookies.is_empty());
        assert!(loaded.deleted_cookies.is_empty());
        assert!(loaded.auth_cookie.is_none());
        assert!(loaded.cookie.is_empty());
        assert!(!directory.join("cookie/default.cookie").exists());
        let stored = fs::read_to_string(&path).unwrap();
        assert!(!stored.contains("selected-secret"));
        assert!(!stored.contains("deleted_cookies"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn deleting_other_cookie_preserves_auth_and_unrelated_external_files() {
        let directory =
            std::env::temp_dir().join(format!("rdnmb-delete-other-{}", uuid::Uuid::new_v4()));
        let path = directory.join("settings.json");
        let mut settings =
            Settings::load_or_initialize_from(&path, "active-secret".into()).unwrap();
        settings
            .cookies
            .insert("other".into(), "other-secret".into());
        settings.save_to(&path).unwrap();
        crate::cookies::store_new_in(&directory.join("cookie"), "external", "external-secret")
            .unwrap();
        remove_saved_cookie(&mut settings, "other");
        settings.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path).unwrap();
        assert_eq!(loaded.cookie, "active-secret");
        assert_eq!(loaded.auth_cookie.as_deref(), Some("default"));
        assert!(!loaded.cookies.contains_key("other"));
        assert_eq!(loaded.cookies["external"], "external-secret");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn removing_an_unsaved_import_does_not_delete_an_external_file() {
        let directory =
            std::env::temp_dir().join(format!("rdnmb-delete-import-{}", uuid::Uuid::new_v4()));
        let path = directory.join("settings.json");
        let mut settings = Settings::load_or_initialize_from(&path, String::new()).unwrap();
        let saved = settings.cookies.clone();
        import_cookie(
            &mut settings.cookies,
            &mut settings.deleted_cookies,
            &mut BTreeMap::new(),
            "new".into(),
            "new-secret".into(),
        )
        .unwrap();
        settings.auth_cookie = Some("new".into());
        remove_cookie(
            &mut settings.cookies,
            &saved,
            &mut settings.deleted_cookies,
            &mut BTreeMap::new(),
            &mut settings.auth_cookie,
            "new",
        );
        assert!(settings.deleted_cookies.is_empty());
        settings.save_to(&path).unwrap();
        assert!(!directory.join("cookie/new.cookie").exists());
        // A file created independently under that name was never scheduled for removal.
        crate::cookies::store_new_in(&directory.join("cookie"), "new", "external-secret").unwrap();
        settings.save_to(&path).unwrap();
        assert_eq!(
            Settings::load_from(&path).unwrap().cookies["new"],
            "external-secret"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn deleted_name_cannot_be_reimported_with_another_value() {
        let mut settings = Settings::default();
        settings
            .cookies
            .insert("saved".into(), "saved-secret".into());
        remove_saved_cookie(&mut settings, "saved");
        let error = import_cookie(
            &mut settings.cookies,
            &mut settings.deleted_cookies,
            &mut BTreeMap::new(),
            "saved".into(),
            "replacement-secret".into(),
        )
        .unwrap_err();
        assert!(!error.to_string().contains("secret"));
        assert!(!settings.cookies.contains_key("saved"));
        assert!(settings.deleted_cookies.contains_key("saved"));
        import_cookie(
            &mut settings.cookies,
            &mut settings.deleted_cookies,
            &mut BTreeMap::new(),
            "saved".into(),
            "saved-secret".into(),
        )
        .unwrap();
        assert!(settings.deleted_cookies.is_empty());
        assert_eq!(settings.cookies["saved"], "saved-secret");
    }

    #[test]
    fn restoring_saved_cookie_cancels_deletion_and_keeps_guest_selection() {
        let directory =
            std::env::temp_dir().join(format!("rdnmb-restore-{}", uuid::Uuid::new_v4()));
        let path = directory.join("settings.json");
        let mut settings = Settings::load_or_initialize_from(&path, "saved-secret".into()).unwrap();
        let saved = settings.cookies.clone();
        let mut removed = BTreeMap::new();
        remove_cookie(
            &mut settings.cookies,
            &saved,
            &mut settings.deleted_cookies,
            &mut removed,
            &mut settings.auth_cookie,
            "default",
        );
        assert_eq!(removed["default"], "saved-secret");
        assert_eq!(settings.deleted_cookies["default"], "saved-secret");
        restore_cookie(
            &mut settings.cookies,
            &mut settings.deleted_cookies,
            &mut removed,
            "default",
        );
        assert!(removed.is_empty());
        assert!(settings.deleted_cookies.is_empty());
        assert_eq!(settings.cookies["default"], "saved-secret");
        assert!(settings.auth_cookie.is_none());
        settings.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path).unwrap();
        assert_eq!(loaded.cookies["default"], "saved-secret");
        assert!(loaded.auth_cookie.is_none());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn removed_unsaved_import_can_be_restored_and_still_rejects_name_conflicts() {
        let mut cookies = BTreeMap::new();
        let saved = BTreeMap::new();
        let mut deleted = BTreeMap::new();
        let mut removed = BTreeMap::new();
        let mut auth = Some("new".into());
        import_cookie(
            &mut cookies,
            &mut deleted,
            &mut removed,
            "new".into(),
            "new-secret".into(),
        )
        .unwrap();
        remove_cookie(
            &mut cookies,
            &saved,
            &mut deleted,
            &mut removed,
            &mut auth,
            "new",
        );
        assert!(cookies.is_empty());
        assert!(deleted.is_empty());
        assert_eq!(removed["new"], "new-secret");
        assert!(auth.is_none());
        let error = import_cookie(
            &mut cookies,
            &mut deleted,
            &mut removed,
            "new".into(),
            "replacement-secret".into(),
        )
        .unwrap_err();
        assert!(!error.to_string().contains("secret"));
        assert_eq!(removed["new"], "new-secret");
        restore_cookie(&mut cookies, &mut deleted, &mut removed, "new");
        assert_eq!(cookies["new"], "new-secret");
        assert!(removed.is_empty());
        assert!(deleted.is_empty());
        // Restoring an import does not make it a saved file on a later removal.
        remove_cookie(
            &mut cookies,
            &saved,
            &mut deleted,
            &mut removed,
            &mut auth,
            "new",
        );
        assert!(deleted.is_empty());
        // Importing the original value is also an explicit restoration.
        import_cookie(
            &mut cookies,
            &mut deleted,
            &mut removed,
            "new".into(),
            "new-secret".into(),
        )
        .unwrap();
        assert_eq!(cookies["new"], "new-secret");
        assert!(removed.is_empty());
        assert!(deleted.is_empty());
    }

    #[test]
    fn changed_cookie_blocks_deletion_without_publishing_settings() {
        let directory =
            std::env::temp_dir().join(format!("rdnmb-delete-conflict-{}", uuid::Uuid::new_v4()));
        let path = directory.join("settings.json");
        let mut settings =
            Settings::load_or_initialize_from(&path, "original-secret".into()).unwrap();
        let previous_settings = fs::read(&path).unwrap();
        remove_saved_cookie(&mut settings, "default");
        let cookie_path = directory.join("cookie/default.cookie");
        fs::write(&cookie_path, "replacement-secret").unwrap();
        let error = settings.save_to(&path).unwrap_err();
        assert!(!format!("{error:#}").contains("secret"));
        assert_eq!(
            fs::read_to_string(&cookie_path).unwrap(),
            "replacement-secret"
        );
        assert_eq!(fs::read(&path).unwrap(), previous_settings);
        assert!(settings.deleted_cookies.contains_key("default"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn failed_settings_publish_restores_removals_and_rolls_back_imports() {
        let directory =
            std::env::temp_dir().join(format!("rdnmb-delete-rollback-{}", uuid::Uuid::new_v4()));
        let path = directory.join("settings.json");
        let mut settings =
            Settings::load_or_initialize_from(&path, "original-secret".into()).unwrap();
        remove_saved_cookie(&mut settings, "default");
        settings.cookies.insert("new".into(), "new-secret".into());
        // A directory cannot be replaced by the prepared settings file.
        let blocked = directory.join("blocked.json");
        fs::create_dir(&blocked).unwrap();
        assert!(settings.save_to(&blocked).is_err());
        assert_eq!(
            Settings::load_from(&path).unwrap().cookie,
            "original-secret"
        );
        assert!(!directory.join("cookie/new.cookie").exists());
        assert!(settings.deleted_cookies.contains_key("default"));
        settings.save_to(&path).unwrap();
        let loaded = Settings::load_from(&path).unwrap();
        assert!(loaded.cookie.is_empty());
        assert_eq!(loaded.cookies["new"], "new-secret");
        assert!(!loaded.cookies.contains_key("default"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rebrand_migration_preserves_credentials_preferences_and_legacy_file() {
        let directory =
            std::env::temp_dir().join(format!("rdnmb-rebrand-{}", uuid::Uuid::new_v4()));
        let legacy = directory.join("nmbxd/settings.json");
        let current = directory.join("rdnmb/settings.json");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        let bytes = br#"{"cookie":"saved-cookie","subscription_id":"existing-shared-subscription","font_family":"Custom Font","website_base":"https://island.example/","ui_font_size":19,"thread_font_size":17,"body_font_size":21}"#;
        fs::write(&legacy, bytes).unwrap();
        let migrated =
            Settings::load_or_initialize_paths(&current, Some(&legacy), "ignored".into()).unwrap();
        assert_eq!(migrated.cookie, "saved-cookie");
        assert_eq!(migrated.auth_cookie.as_deref(), Some("default"));
        assert_eq!(migrated.cookies["default"], "saved-cookie");
        assert_eq!(migrated.subscription_id, "existing-shared-subscription");
        assert_eq!(migrated.font_family, "Custom Font");
        assert_eq!(migrated.website_base, "https://island.example/");
        assert_eq!(
            (
                migrated.ui_font_size,
                migrated.thread_font_size,
                migrated.body_font_size
            ),
            (19., 17., 21.)
        );
        assert_eq!(fs::read(&legacy).unwrap(), bytes);
        assert_eq!(
            fs::read(current.parent().unwrap().join("cookie/default.cookie")).unwrap(),
            b"saved-cookie"
        );
        let stored: serde_json::Value =
            serde_json::from_slice(&fs::read(&current).unwrap()).unwrap();
        assert!(stored.get("cookie").is_none());
        assert!(stored.get("cookies").is_none());
        assert_eq!(
            Settings::load_from(&current).unwrap().cookie,
            "saved-cookie"
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn migration_respects_current_settings_and_reports_invalid_legacy_without_writes() {
        let directory =
            std::env::temp_dir().join(format!("rdnmb-rebrand-errors-{}", uuid::Uuid::new_v4()));
        let legacy = directory.join("nmbxd/settings.json");
        let current = directory.join("rdnmb/settings.json");
        fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        for bytes in [
            b"{invalid".as_slice(),
            br#"{"cookie":null}"#.as_slice(),
            br#"{"subscription_id":""}"#.as_slice(),
            br#"{"cookie":"unsafe\r\nvalue"}"#.as_slice(),
        ] {
            fs::write(&legacy, bytes).unwrap();
            assert!(
                Settings::load_or_initialize_paths(&current, Some(&legacy), "fallback".into())
                    .is_err()
            );
            assert!(!current.exists());
            assert!(!current.parent().unwrap().join("cookie").exists());
            assert_eq!(fs::read(&legacy).unwrap(), bytes);
        }
        let original = Settings::default();
        original.save_to(&current).unwrap();
        let current_bytes = fs::read(&current).unwrap();
        let loaded =
            Settings::load_or_initialize_paths(&current, Some(&legacy), "ignored".into()).unwrap();
        assert!(loaded.cookie.is_empty());
        assert_eq!(loaded.subscription_id, original.subscription_id);
        assert_eq!(fs::read(&current).unwrap(), current_bytes);
        fs::write(&current, b"{invalid-current").unwrap();
        assert!(
            Settings::load_or_initialize_paths(&current, Some(&legacy), "ignored".into()).is_err()
        );
        assert_eq!(fs::read(&current).unwrap(), b"{invalid-current");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn migration_preserves_existing_default_and_legacy_value_without_overwrite() {
        let directory =
            std::env::temp_dir().join(format!("rdnmb-cookie-conflict-{}", uuid::Uuid::new_v4()));
        let path = directory.join("settings.json");
        crate::cookies::store_new_in(&directory.join("cookie"), "default", "existing-cookie")
            .unwrap();
        fs::write(&path, br#"{"cookie":"legacy-cookie"}"#).unwrap();
        let loaded = Settings::load_or_initialize_from(&path, "ignored".into()).unwrap();
        assert_eq!(loaded.cookies["default"], "existing-cookie");
        assert_eq!(loaded.cookies["legacy-1"], "legacy-cookie");
        assert_eq!(loaded.auth_cookie.as_deref(), Some("legacy-1"));
        assert_eq!(loaded.cookie, "legacy-cookie");
        let reloaded = Settings::load_or_initialize_from(&path, "ignored".into()).unwrap();
        assert_eq!(loaded.cookies, reloaded.cookies);
        fs::remove_dir_all(directory).unwrap();
    }

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
            "rdnmb-settings-startup-test-{}",
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
            "rdnmb-settings-invalid-test-{}",
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
            "rdnmb-settings-migration-test-{}-{}",
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
            "rdnmb-settings-subscription-test-{}",
            uuid::Uuid::new_v4()
        ));
        let path = directory.join("settings.json");
        let mut settings = Settings {
            subscription_id: "   ".into(),
            ..Settings::default()
        };
        assert!(settings.save_to(&path).is_err());
        assert!(!directory.exists());
        settings.subscription_id = "岛".repeat(rdnmb::api::MAX_SUBSCRIPTION_ID_CHARS + 1);
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
            "rdnmb-settings-size-test-{}-{}",
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
    fn guest_selection_persists_and_keeps_all_cookie_files() {
        let directory = std::env::temp_dir().join(format!("rdnmb-guest-{}", uuid::Uuid::new_v4()));
        let path = directory.join("settings.json");
        let mut settings =
            Settings::load_or_initialize_from(&path, "userhash=test-only".into()).unwrap();
        settings
            .cookies
            .insert("second".into(), "userhash=second-only".into());
        settings.auth_cookie = Some("second".into());
        settings.save_to(&path).unwrap();
        assert_eq!(
            Settings::load_from(&path).unwrap().cookie,
            "userhash=second-only"
        );
        settings.auth_cookie = None;
        settings.save_to(&path).unwrap();
        let loaded = Settings::load_or_initialize_from(&path, "ignored-fallback".into()).unwrap();
        assert!(loaded.cookie.is_empty());
        assert_eq!(loaded.auth_cookie, None);
        assert_eq!(loaded.cookies.len(), 2);
        let stored = fs::read_to_string(&path).unwrap();
        assert!(!stored.contains("test-only"));
        assert!(!stored.contains("second-only"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn empty_legacy_cookie_suppresses_fallback_and_default_file() {
        let directory =
            std::env::temp_dir().join(format!("rdnmb-legacy-guest-{}", uuid::Uuid::new_v4()));
        let path = directory.join("settings.json");
        crate::cookies::store_new_in(&directory.join("cookie"), "default", "existing-cookie")
            .unwrap();
        fs::write(&path, br#"{"cookie":""}"#).unwrap();
        let loaded = Settings::load_or_initialize_from(&path, "fallback-cookie".into()).unwrap();
        assert!(loaded.cookie.is_empty());
        assert_eq!(loaded.auth_cookie, None);
        assert_eq!(loaded.cookies["default"], "existing-cookie");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn mixed_schema_preserves_legacy_cookie_without_changing_explicit_guest() {
        let directory = std::env::temp_dir().join(format!("rdnmb-mixed-{}", uuid::Uuid::new_v4()));
        let path = directory.join("settings.json");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, br#"{"cookie":"legacy-cookie","auth_cookie":null}"#).unwrap();
        let loaded = Settings::load_or_initialize_from(&path, "ignored".into()).unwrap();
        assert!(loaded.cookie.is_empty());
        assert_eq!(loaded.cookies["default"], "legacy-cookie");
        assert!(!fs::read_to_string(&path).unwrap().contains("legacy-cookie"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn removed_auth_cookie_starts_as_guest_and_preserves_other_cookies() {
        let directory =
            std::env::temp_dir().join(format!("rdnmb-missing-auth-{}", uuid::Uuid::new_v4()));
        let path = directory.join("settings.json");
        crate::cookies::store_new_in(&directory.join("cookie"), "other", "other-cookie").unwrap();
        fs::write(
            &path,
            br#"{"auth_cookie":"removed-cookie","subscription_id":"existing-id"}"#,
        )
        .unwrap();
        let loaded = Settings::load_or_initialize_from(&path, "ignored".into()).unwrap();
        assert!(loaded.cookie.is_empty());
        assert!(loaded.auth_cookie.is_none());
        assert_eq!(loaded.cookies.len(), 1);
        fs::remove_dir_all(directory).unwrap();
    }
}
