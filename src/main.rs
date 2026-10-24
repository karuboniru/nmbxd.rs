#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod appearance;
mod assets;
mod attachments;
mod cookies;
mod drafts;
mod emoji;
#[cfg(not(target_os = "linux"))]
mod file_dialog;
mod history_controller;
mod history_view;
mod image_actions;
mod image_clipboard;
mod image_export;
mod kaomoji;
mod kaomoji_picker;
mod media;
mod media_view;
mod navigation;
mod panes;
mod preview;
mod preview_shortcuts;
mod reply_cookies;
mod scrolling;
mod selectable_content;
mod settings;
mod subscriptions;
mod timeline_navigation;
use appearance::{Appearance, Palette};
use gpui::{prelude::*, *};
use gpui_component::{
    Disableable, Root,
    button::Button,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, DropdownMenu, PopupMenuItem},
};
use navigation::{Navigation, PageAction, PageMode, parse_page};
use rdnmb::{
    APP_ID, APP_NAME,
    api::ApiClient,
    config_paths,
    history::{HistoryCategory, HistoryEntry, HistoryStore, PostingEntry, ReadingPosition},
    model::{ForumGroup, Post, Timeline},
    tree::ThreadTree,
};
use scrolling::ScrollPaging;
use settings::{Settings, SettingsEvent, SettingsPanel};
use std::{collections::HashSet, sync::mpsc, time::Duration};

enum Request {
    Feed(u64, String, u32),
    FeedChange(String, u64, bool),
    Reference(u64, u64, std::sync::Weak<()>),
    JumpProbe(u64, u64, std::sync::Weak<()>),
    Configure(Settings),
    Forums,
    Timelines,
    Timeline(u64, i64, u32),
    Forum(u64, i64, u32),
    Thread(u64, u64, u32),
    Send(
        Option<u64>,
        i64,
        String,
        String,
        Option<rdnmb::api::ImageAttachment>,
        String,
        String,
    ),
}
enum Response {
    FeedChanged(String, u64, bool, Result<String, String>),
    Reference(u64, Result<rdnmb::api::ReferencePreview, String>),
    JumpProbe(u64, Result<Post, String>),
    Configured(Result<Settings, String>),
    Forums(Result<Vec<ForumGroup>, String>),
    Timelines(Result<Vec<Timeline>, String>),
    Forum(u64, u32, Result<Vec<Post>, String>),
    Thread(u64, u32, Result<Post, String>),
    Sent(Result<String, String>, Option<String>, Option<u64>, String),
}

struct ComposerDraft {
    cookie: Option<String>,
    body: Entity<InputState>,
    title: Entity<InputState>,
    attachment: Option<attachments::DraftImage>,
}

struct Island {
    reply_cookies: reply_cookies::ReplyCookies,
    compose_cookie: Option<String>,
    history: Option<HistoryStore>,
    showing_history: bool,
    history_category: HistoryCategory,
    history_entries: Vec<HistoryEntry>,
    history_offset: u32,
    history_has_more: bool,
    history_scroll: ScrollHandle,
    reading_restore: Option<ReadingPosition>,
    restore_scheduled: bool,
    position_save_revision: u64,
    drafts: drafts::Drafts<ComposerDraft>,
    image_export_busy: bool,
    content_views: std::cell::RefCell<
        std::collections::VecDeque<(
            ElementId,
            String,
            Entity<selectable_content::SelectableContent>,
        )>,
    >,
    preview_shortcut_subscription: Option<Subscription>,
    showing_subscriptions: bool,
    subscribed: HashSet<u64>,
    subscription_pending: HashSet<u64>,
    previews: Vec<preview::Frame>,
    preview_serial: u64,
    settings: Settings,
    media: Entity<media::MediaStore>,
    viewing_image: Option<preview::ImageViewer>,
    settings_panel: Option<Entity<SettingsPanel>>,
    settings_saving: bool,
    appearance: Appearance,
    title_drag: bool,
    tx: mpsc::Sender<Request>,
    groups: Vec<ForumGroup>,
    timelines: Vec<Timeline>,
    timeline: Option<Timeline>,
    timelines_loading: bool,
    timelines_error: Option<String>,
    forum: i64,
    forum_name: String,
    threads: Vec<Post>,
    forum_page: u32,
    forum_epoch: u64,
    forum_scroll: ScrollHandle,
    forum_paging: ScrollPaging,
    forum_end: bool,
    thread_epoch: u64,
    tree: Option<ThreadTree>,
    selected: Option<u64>,
    navigation: Navigation,
    page_input: Entity<InputState>,
    reading_revision: u64,
    reading_scroll: ScrollHandle,
    reading_paging: ScrollPaging,
    thread_end_checked: bool,
    cached_short_check: bool,
    cached_check_scheduled: bool,
    reading_reset_pending: bool,
    reading_retry: Option<(u32, PageMode)>,
    panes: panes::PaneLayout,
    groups_loading: bool,
    forum_loading: bool,
    aggregate: bool,
    collapsed: HashSet<u64>,
    composing: bool,
    kaomoji_open: bool,
    compose_target: Option<u64>,
    compose_forum: i64,
    compose_label: String,
    animation: u64,
    body: Entity<InputState>,
    title: Entity<InputState>,
    sending: bool,
    clear_draft: bool,
    attachment: Option<attachments::DraftImage>,
    attachment_loading: bool,
    attachment_revision: u64,
    cookie: bool,
    notice: String,
}

impl Island {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        api: ApiClient,
        settings: Settings,
    ) -> Self {
        let appearance = Appearance::load();
        appearance.apply(window, cx);
        apply_fonts(&settings, cx);
        cx.observe_window_appearance(window, |this, window, cx| {
            if this.appearance == Appearance::System {
                this.appearance.apply(window, cx);
                apply_fonts(&this.settings, cx);
                cx.notify();
            }
        })
        .detach();
        let (history, history_error) = match HistoryStore::open_default() {
            Ok(store) => (Some(store), None),
            Err(error) => (None, Some(format!("本地历史不可用：{error}"))),
        };
        cx.on_release(|this, _| this.save_reading_position())
            .detach();
        cx.on_app_quit(|this, _| {
            this.save_reading_position();
            async {}
        })
        .detach();
        let posting_history = history.clone();
        let cookie = api.has_cookie();
        let (tx, rx) = mpsc::channel();
        let (out, incoming) = mpsc::channel();
        std::thread::spawn(move || {
            let mut api = api;
            while let Ok(request) = rx.recv() {
                let result = match request {
                    Request::Feed(g, uuid, page) => Response::Forum(
                        g,
                        page,
                        api.subscriptions(&uuid, page).map_err(|e| e.to_string()),
                    ),
                    Request::FeedChange(uuid, id, add) => {
                        let result = if add {
                            api.add_subscription(&uuid, id)
                        } else {
                            api.remove_subscription(&uuid, id)
                        };
                        Response::FeedChanged(uuid, id, add, result.map_err(|e| e.to_string()))
                    }
                    Request::Configure(mut settings) => {
                        let result = (|| -> anyhow::Result<Settings> {
                            let client = ApiClient::with_website_base(
                                Some(&settings.cookie),
                                &settings.website_base,
                            )?;
                            settings.website_base = client.website_base().to_owned();
                            settings.save()?;
                            settings.deleted_cookies.clear();
                            api = client;
                            Ok(settings)
                        })()
                        .map_err(|error| error.to_string());
                        Response::Configured(result)
                    }
                    Request::Reference(g, id, lifetime) => {
                        if lifetime.upgrade().is_none() {
                            continue;
                        }
                        Response::Reference(g, api.reference(id).map_err(|e| e.to_string()))
                    }
                    Request::JumpProbe(g, id, lifetime) => {
                        if lifetime.upgrade().is_none() {
                            continue;
                        }
                        Response::JumpProbe(g, api.thread_page(id, 1).map_err(|e| e.to_string()))
                    }
                    Request::Forums => {
                        Response::Forums(api.forum_groups().map_err(|e| e.to_string()))
                    }
                    Request::Timelines => {
                        Response::Timelines(api.timelines().map_err(|e| e.to_string()))
                    }
                    Request::Timeline(g, id, page) => Response::Forum(
                        g,
                        page,
                        api.timeline_page(id, page).map_err(|e| e.to_string()),
                    ),
                    Request::Forum(g, id, page) => Response::Forum(
                        g,
                        page,
                        api.forum_page(id, page).map_err(|e| e.to_string()),
                    ),
                    Request::Thread(g, id, page) => Response::Thread(
                        g,
                        page,
                        api.thread_page(id, page).map_err(|e| e.to_string()),
                    ),
                    Request::Send(target, fid, title, body, image, cookie_name, cookie_value) => {
                        let result =
                            ApiClient::with_website_base(Some(&cookie_value), api.website_base())
                                .and_then(|posting_api| match target {
                                    Some(id) => posting_api.reply_with_image(
                                        id,
                                        "",
                                        &title,
                                        &body,
                                        image.as_ref(),
                                    ),
                                    None => posting_api.post_thread_with_image(
                                        fid,
                                        "",
                                        &title,
                                        &body,
                                        image.as_ref(),
                                    ),
                                })
                                .map_err(|e| e.to_string());
                        // Capture the exact submitted draft, not the current editor state.
                        let history_error = if let Ok(message) = &result {
                            if let Some(store) = &posting_history {
                                store
                                    .record_post(&PostingEntry {
                                        id: 0,
                                        target,
                                        forum_id: fid,
                                        title,
                                        body,
                                        has_image: image.is_some(),
                                        sent_at: rdnmb::history::now_millis(),
                                        message: message.clone(),
                                    })
                                    .err()
                                    .map(|error| error.to_string())
                            } else {
                                Some("本地历史不可用".into())
                            }
                        } else {
                            None
                        };
                        Response::Sent(result, history_error, target, cookie_name)
                    }
                };
                if out.send(result).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(800));
            }
        });
        cx.spawn(async move |view, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(80))
                    .await;
                if view
                    .update(cx, |this, cx| {
                        while let Ok(response) = incoming.try_recv() {
                            this.receive(response, cx);
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
        tx.send(Request::Forums).ok();
        tx.send(Request::Timelines).ok();
        tx.send(Request::Forum(1, 4, 1)).ok();
        let page_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("页码")
                .validate(|value, _| {
                    value.len() <= 10 && value.bytes().all(|byte| byte.is_ascii_digit())
                })
        });
        cx.subscribe(&page_input, |this, _, event, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.jump_to_page(cx);
            }
        })
        .detach();
        let panes = panes::PaneLayout::new(
            settings.pane_layout.clone(),
            |this: &mut Self, layout, cx| {
                this.settings.pane_layout = Some(layout.clone());
                if let Err(error) = this.settings.save() {
                    this.notice = format!("布局保存失败：{error}");
                    cx.notify();
                }
            },
            cx,
        );
        let media = cx.new(|_| media::MediaStore::new());
        cx.observe(&media, |_, _, cx| cx.notify()).detach();
        let (reply_cookies, cookie_memory_error) = match reply_cookies::ReplyCookies::load() {
            Ok(memory) => (memory, None),
            Err(error) => (
                Default::default(),
                Some(format!("回复饼干记录不可用：{error}")),
            ),
        };
        Self {
            reply_cookies,
            compose_cookie: None,
            history,
            showing_history: false,
            history_category: HistoryCategory::Browsing,
            history_entries: Vec::new(),
            history_offset: 0,
            history_has_more: false,
            history_scroll: ScrollHandle::new(),
            reading_restore: None,
            restore_scheduled: false,
            position_save_revision: 0,
            image_export_busy: false,
            content_views: Default::default(),
            preview_shortcut_subscription: None,
            showing_subscriptions: false,
            subscribed: HashSet::new(),
            subscription_pending: HashSet::new(),
            previews: Vec::new(),
            preview_serial: 0,
            settings,
            media,
            viewing_image: None,
            settings_panel: None,
            settings_saving: false,
            appearance,
            title_drag: false,
            tx,
            groups: vec![],
            timelines: vec![],
            timeline: None,
            timelines_loading: true,
            timelines_error: None,
            forum: 4,
            forum_name: "综合版1".into(),
            threads: vec![],
            forum_page: 0,
            forum_epoch: 1,
            forum_scroll: ScrollHandle::new(),
            forum_paging: ScrollPaging::default(),
            forum_end: false,
            thread_epoch: 0,
            tree: None,
            selected: None,
            navigation: Navigation::default(),
            page_input,
            reading_revision: 0,
            reading_scroll: ScrollHandle::new(),
            reading_paging: ScrollPaging::default(),
            thread_end_checked: false,
            cached_short_check: false,
            cached_check_scheduled: false,
            reading_reset_pending: false,
            reading_retry: None,
            panes,
            groups_loading: true,
            forum_loading: true,
            aggregate: false,
            collapsed: HashSet::new(),
            composing: false,
            kaomoji_open: false,
            drafts: drafts::Drafts::default(),
            compose_target: None,
            compose_forum: 4,
            compose_label: String::new(),
            animation: 0,
            body: cx.new(|cx| {
                InputState::new(window, cx)
                    .multi_line(true)
                    .placeholder("在这里写下想说的话…")
            }),
            title: cx.new(|cx| InputState::new(window, cx).placeholder("标题（可选）")),
            sending: false,
            clear_draft: false,
            attachment: None,
            attachment_loading: false,
            attachment_revision: 0,
            cookie,
            notice: history_error
                .or(cookie_memory_error)
                .unwrap_or_else(|| "正在连接岛屿…".into()),
        }
    }

    fn receive(&mut self, response: Response, cx: &mut Context<Self>) {
        match response {
            Response::FeedChanged(uuid, id, add, result) => {
                self.receive_subscription(uuid, id, add, result, cx)
            }
            Response::Reference(g, result) => self.receive_preview(g, result),
            Response::JumpProbe(g, result) => self.receive_jump(g, result, cx),
            Response::Configured(result) => {
                self.settings_saving = false;
                match result {
                    Ok(settings) => {
                        self.cookie = !settings.cookie.trim().is_empty();
                        apply_fonts(&settings, cx);
                        let subscription_changed =
                            self.settings.subscription_id != settings.subscription_id;
                        self.settings = settings;
                        if subscription_changed {
                            self.subscribed.clear();
                            self.subscription_pending.clear();
                            if self.showing_subscriptions {
                                self.load_subscriptions(cx);
                            }
                        }
                        self.settings_panel = None;
                        self.notice = "设置已保存，后续请求将使用新设置".into();
                        cx.refresh_windows();
                    }
                    Err(error) => {
                        if let Some(panel) = &self.settings_panel {
                            panel.update(cx, |panel, cx| panel.set_error(error, cx));
                        }
                    }
                }
            }
            Response::Forums(result) => {
                self.groups_loading = false;
                match result {
                    Ok(groups) => self.groups = groups,
                    Err(e) => self.notice = e,
                }
            }
            Response::Timelines(result) => {
                self.timelines_loading = false;
                match result {
                    Ok(timelines) => {
                        // Refresh metadata without changing the active list or its generation.
                        if let Some(selected) = &mut self.timeline
                            && let Some(updated) =
                                timelines.iter().find(|item| item.id == selected.id)
                        {
                            *selected = updated.clone();
                        }
                        self.timelines = timelines;
                        self.timelines_error = None;
                    }
                    Err(error) => self.timelines_error = Some(error),
                }
            }
            Response::Forum(g, page, result) if g == self.forum_epoch => {
                self.forum_loading = false;
                match result {
                    Ok(posts) => {
                        if self.showing_subscriptions {
                            if page == 1 {
                                self.subscribed.clear();
                            }
                            self.subscribed.extend(posts.iter().map(|p| p.id));
                        }
                        if page == 1 {
                            self.threads.clear();
                        }
                        let previous_len = self.threads.len();
                        let returned_empty = posts.is_empty();
                        for post in posts {
                            if !self.threads.iter().any(|p| p.id == post.id) {
                                self.threads.push(post);
                            }
                        }
                        // Forum pages have no total; pinned posts may repeat and
                        // page size alone cannot establish the end.
                        self.forum_page = page;
                        self.forum_end = self.timeline_limit_reached(page)
                            || if self.showing_subscriptions {
                                returned_empty
                            } else {
                                self.threads.len() == previous_len
                            };
                        self.forum_paging.failed = false;
                        self.notice = "慢慢浏览，自在交流".into();
                    }
                    Err(e) => {
                        self.forum_paging.failed = true;
                        self.notice = e;
                    }
                }
            }
            Response::Thread(g, page, result) if g == self.thread_epoch => {
                self.receive_thread_page(page, result, false, cx);
            }
            Response::Sent(result, history_error, target, cookie_name) => {
                self.sending = false;
                match result {
                    Ok(message) => {
                        self.notice = if let Some(error) = history_error {
                            format!("发送成功，但本地发帖历史保存失败：{error}")
                        } else {
                            format!("发送结果：{message} · 已保存发帖历史")
                        };
                        if let Err(error) = self.reply_cookies.record(target, &cookie_name) {
                            self.notice
                                .push_str(&format!(" · 回复饼干记录保存失败：{error}"));
                        }
                        self.compose_cookie = None;
                        self.composing = false;
                        self.kaomoji_open = false;
                        self.clear_draft = true;
                    }
                    Err(e) => {
                        self.notice = format!("发送未确认，草稿已保留。请先检查串再重试：{e}");
                        self.reader_target_changed();
                    }
                }
            }
            _ => {}
        }
        cx.notify();
    }

    fn load_forum(&mut self, id: i64, name: String, cx: &mut Context<Self>) {
        self.showing_history = false;
        self.showing_subscriptions = false;
        self.timeline = None;
        self.forum_loading = false;
        self.forum = id;
        self.forum_name = name;
        self.reader_target_changed();
        self.forum_epoch += 1;
        self.threads.clear();
        self.forum_page = 0;
        self.forum_end = false;
        self.forum_scroll = ScrollHandle::new();
        self.forum_paging = ScrollPaging::default();
        self.request_forum_next(cx);
    }
    fn open_thread(&mut self, id: u64, cx: &mut Context<Self>) {
        self.open_saved_thread(id, cx);
    }

    fn request_page(&mut self, page: u32, mode: PageMode, cx: &mut Context<Self>) {
        let Some(id) = self.selected else {
            return;
        };
        if matches!(mode, PageMode::Jump | PageMode::Refresh) {
            self.save_reading_position();
        }
        match self.navigation.request(page, mode) {
            PageAction::Ignored => return,
            PageAction::Cached => {
                self.tree = self.navigation.tree();
                if matches!(mode, PageMode::Jump | PageMode::Refresh) {
                    self.reading_restore = None;
                    self.reset_reading_scroll();
                }
                self.reading_retry = None;
                self.reading_paging.failed = false;
                self.notice = format!("第 {page} 页 · 使用已加载内容");
            }
            PageAction::Fetch => {
                self.reading_reset_pending = matches!(mode, PageMode::Jump | PageMode::Refresh);
                if !self.reading_reset_pending {
                    let cached = self
                        .history
                        .as_ref()
                        .and_then(|store| store.cached_page(id, page).ok().flatten());
                    if !matches!(mode, PageMode::Recheck)
                        && let Some(post) = cached
                    {
                        self.receive_thread_page(page, Ok(post), true, cx);
                        return;
                    }
                }
                self.reading_retry = Some((page, mode));
                self.reading_paging.failed = false;
                self.notice = format!("正在加载第 {page} 页…");
                if self
                    .tx
                    .send(Request::Thread(self.thread_epoch, id, page))
                    .is_err()
                {
                    self.navigation.receive(page, None);
                    self.reading_reset_pending = false;
                    self.reading_paging.failed = true;
                    self.notice = "后台服务不可用，请重启客户端".into();
                }
            }
        }
        cx.notify();
    }

    fn reset_reading_scroll(&mut self) {
        self.reading_revision += 1;
        self.reading_scroll = ScrollHandle::new();
        self.reading_paging.check_pending = false;
    }

    fn request_forum_next(&mut self, cx: &mut Context<Self>) {
        if self.forum_loading || self.forum_end {
            return;
        }
        let Some(page) = self.forum_page.checked_add(1) else {
            self.forum_end = true;
            return;
        };
        if self.timeline_limit_reached(self.forum_page) {
            self.forum_end = true;
            cx.notify();
            return;
        }
        self.forum_loading = true;
        self.forum_paging.failed = false;
        let request = if self.showing_subscriptions {
            Request::Feed(
                self.forum_epoch,
                self.settings.subscription_id.clone(),
                page,
            )
        } else if let Some(timeline) = &self.timeline {
            Request::Timeline(self.forum_epoch, timeline.id, page)
        } else {
            Request::Forum(self.forum_epoch, self.forum, page)
        };
        if self.tx.send(request).is_err() {
            self.forum_loading = false;
            self.forum_paging.failed = true;
            self.notice = "后台服务不可用，请重启客户端".into();
        }
        cx.notify();
    }

    fn reading_next(&self) -> Option<(u32, PageMode)> {
        if self.navigation.current() == 0 {
            return Some((1, PageMode::Navigate));
        }
        self.navigation.after().map(|page| (page, PageMode::Buffer))
    }

    fn scroll_forum(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(20.)).y;
        let downward = delta < px(0.);
        if delta == px(0.)
            || self.forum_loading
            || self.forum_end
            || self.forum_paging.check_pending
        {
            return;
        }
        self.forum_paging.check_pending = true;
        let epoch = self.forum_epoch;
        // The built-in wheel listener applies the scroll offset during this
        // event. Inspect it after dispatch, rather than using the old offset.
        cx.defer_in(window, move |this, _, cx| {
            if this.forum_epoch != epoch {
                return;
            }
            this.forum_paging.check_pending = false;
            let at_bottom =
                this.forum_scroll.offset().y + this.forum_scroll.max_offset().height <= px(2.);
            if !this.forum_loading
                && !this.forum_end
                && this.forum_paging.on_scroll(at_bottom, downward)
            {
                this.request_forum_next(cx);
            }
            cx.notify();
        });
    }

    fn scroll_reading(
        &mut self,
        event: &ScrollWheelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(20.)).y;
        let downward = delta < px(0.);
        if delta == px(0.) || self.selected.is_none() || self.reading_paging.check_pending {
            return;
        }
        self.reading_paging.check_pending = true;
        let revision = self.reading_revision;
        cx.defer_in(window, move |this, _, cx| {
            if this.reading_revision != revision {
                return;
            }
            this.reading_paging.check_pending = false;
            this.schedule_position_save(cx);
            let at_bottom =
                this.reading_scroll.offset().y + this.reading_scroll.max_offset().height <= px(2.);
            if !this.navigation.loading() {
                let load_next = this.reading_paging.on_scroll(at_bottom, downward);
                if let Some((page, mode)) = this.reading_next() {
                    if load_next {
                        this.request_page(page, mode, cx);
                    }
                } else if at_bottom && downward {
                    this.check_thread_end_once(cx);
                }
            }
            cx.notify();
        });
    }

    fn jump_to_page(&mut self, cx: &mut Context<Self>) {
        if self.navigation.loading() || self.navigation.current() == 0 {
            return;
        }
        let value = self.page_input.read(cx).value();
        if let Some(page) = parse_page(value.as_ref(), self.navigation.total()) {
            self.request_page(page, PageMode::Jump, cx);
        } else {
            self.notice = format!("请输入 1–{} 之间的整数页码", self.navigation.total());
            cx.notify();
        }
    }

    fn clear_current_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.clear_draft {
            self.attachment = None;
            self.attachment_loading = false;
            self.attachment_revision += 1;
            self.body
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.title
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.clear_draft = false;
        }
    }

    pub(crate) fn reader_target_changed(&mut self) {
        if !drafts::Target::new(self.compose_target, self.compose_forum).can_send(
            self.selected,
            timeline_navigation::posting_forum(self.timeline.as_ref(), self.forum),
        ) {
            self.composing = false;
            self.kaomoji_open = false;
        }
    }

    fn compose(
        &mut self,
        target: Option<u64>,
        quote: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sending {
            return;
        }
        self.clear_current_draft(window, cx);
        let from = drafts::Target::new(self.compose_target, self.compose_forum);
        let forum = timeline_navigation::posting_forum(self.timeline.as_ref(), self.forum);
        let to = drafts::Target::new(target, forum);
        if from != to {
            // In-flight file/clipboard reads must never attach to the next draft.
            self.attachment_revision += 1;
            self.attachment_loading = false;
            let current = ComposerDraft {
                cookie: self.compose_cookie.take(),
                body: self.body.clone(),
                title: self.title.clone(),
                attachment: self.attachment.take(),
            };
            let next = self.drafts.switch(from, to, current, || ComposerDraft {
                cookie: None,
                body: cx.new(|cx| {
                    InputState::new(window, cx)
                        .multi_line(true)
                        .placeholder("在这里写下想说的话…")
                }),
                title: cx.new(|cx| InputState::new(window, cx).placeholder("标题（可选）")),
                attachment: None,
            });
            self.compose_cookie = next.cookie;
            self.body = next.body;
            self.title = next.title;
            self.attachment = next.attachment;
            self.notice = "已切换到此目标的独立草稿；其他串的草稿已保留（本次运行内）".into();
        }
        self.compose_target = target;
        self.compose_forum = forum;
        self.compose_label = target.map(|id| format!("回复 No.{id}")).unwrap_or_else(|| {
            let name = if self.timeline.is_some() {
                "综合版1"
            } else {
                &self.forum_name
            };
            format!("发串到 {name}")
        });
        if let Some(id) = quote {
            let current = self.body.read(cx).value();
            let suffix = drafts::quote_suffix(&current, id);
            let end = current.encode_utf16().count();
            self.body.update(cx, |input, cx| {
                // Native insertion keeps undo history and puts the caret after
                // the quote's trailing newline (set_value resets it to start).
                input.replace_text_in_range(Some(end..end), &suffix, window, cx);
            });
        }
        self.kaomoji_open = false;
        self.composing = true;
        self.animation += 1;
        self.body.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
    }
    fn posting_cookie_name(&self) -> Option<&str> {
        self.compose_cookie
            .as_deref()
            .filter(|name| self.settings.cookies.contains_key(*name))
            .or_else(|| {
                self.reply_cookies.selected(
                    self.compose_target,
                    self.settings.auth_cookie.as_deref(),
                    &self.settings.cookies,
                )
            })
    }

    fn cookie_dropdown(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.posting_cookie_name().map(str::to_owned);
        let names: Vec<_> = self.settings.cookies.keys().cloned().collect();
        let view = cx.entity();
        Button::new("posting-cookie")
            .label(format!(
                "回复 / 发串饼干：{} ▾",
                selected.as_deref().unwrap_or("请选择")
            ))
            .disabled(self.sending)
            .dropdown_menu(move |mut menu, _, _| {
                for name in &names {
                    let chosen = name.clone();
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(selected.as_ref() == Some(name))
                            .on_click(move |_, _, cx| {
                                view.update(cx, |this, cx| {
                                    if !this.sending {
                                        this.compose_cookie = Some(chosen.clone());
                                        cx.notify();
                                    }
                                });
                            }),
                    );
                }
                menu
            })
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        if self.sending || self.attachment_loading || !self.composing {
            return;
        }
        if self.clear_draft
            || !drafts::Target::new(self.compose_target, self.compose_forum).can_send(
                self.selected,
                timeline_navigation::posting_forum(self.timeline.as_ref(), self.forum),
            )
        {
            self.notice = "已阻止发送：草稿目标与当前浏览目标不一致，请在目标串重新点击回复".into();
            cx.notify();
            return;
        }
        let Some(cookie_name) = self.posting_cookie_name().map(str::to_owned) else {
            self.notice = "请先导入饼干，并在回复框选择用于发送的饼干".into();
            cx.notify();
            return;
        };
        let cookie_value = self.settings.cookies[&cookie_name].clone();
        let body = self.body.read(cx).value().to_string();
        if body.trim().is_empty() && self.attachment.is_none() {
            self.notice = "请填写正文或添加图片后再发送".into();
            cx.notify();
            return;
        }
        self.sending = true;
        if self
            .tx
            .send(Request::Send(
                self.compose_target,
                self.compose_forum,
                self.title.read(cx).value().to_string(),
                body,
                self.attachment
                    .as_ref()
                    .map(|image| image.attachment.clone()),
                cookie_name,
                cookie_value,
            ))
            .is_err()
        {
            self.sending = false;
            self.notice = "后台服务不可用，草稿已保留，请重启客户端".into();
        }
        cx.notify();
    }

    fn choose_image(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sending || self.attachment_loading {
            return;
        }
        self.attachment_loading = true;
        self.attachment_revision += 1;
        let revision = self.attachment_revision;
        let picker = attachments::pick_image(window);
        cx.spawn(async move |this, cx| {
            let result = match picker.await {
                Ok(Some(path)) => Some(
                    cx.background_executor()
                        .spawn(async move { attachments::load_file(path) })
                        .await,
                ),
                Ok(None) => None,
                Err(error) => Some(Err(error)),
            };
            let _ = this.update(cx, |this, cx| {
                if this.attachment_revision != revision {
                    return;
                }
                this.attachment_loading = false;
                if let Some(result) = result {
                    match result {
                        Ok(image) => {
                            this.attachment = Some(image);
                            this.notice = "图片已加入草稿，点击发送才会上传".into();
                        }
                        Err(error) => this.notice = error,
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn paste_image(&mut self, explicit: bool, cx: &mut Context<Self>) -> bool {
        if self.sending || self.attachment_loading {
            return false;
        }
        let Some(image) = cx
            .read_from_clipboard()
            .and_then(attachments::clipboard_image)
        else {
            if explicit {
                self.notice = "剪贴板中没有图片；可复制截图后粘贴，或使用选择图片".into();
                cx.notify();
            }
            return false;
        };
        self.attachment_loading = true;
        self.attachment_revision += 1;
        let revision = self.attachment_revision;
        let task = cx
            .background_executor()
            .spawn(async move { attachments::load_clipboard(image) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                if this.attachment_revision != revision {
                    return;
                }
                this.attachment_loading = false;
                match result {
                    Ok(image) => {
                        this.attachment = Some(image);
                        this.notice = "剪贴板图片已加入草稿，点击发送才会上传".into();
                    }
                    Err(error) => this.notice = error,
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
        true
    }

    fn post_card(
        &self,
        post: &Post,
        depth: usize,
        missing: &[u64],
        children: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let p = Palette::current(cx);
        let id = post.id;
        let collapsed = self.collapsed.contains(&id);
        div()
            .ml(px((depth.min(7) * 16) as f32))
            .mb_3()
            .p_4()
            .bg(rgb(p.surface))
            .rounded_lg()
            .border_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .text_xs()
                    .text_color(rgb(p.muted))
                    .child(format!("{}  ·  No.{}", post.user_hash, id))
                    .child(post.now.to_string()),
            )
            .when(post.title != "无标题" && !post.title.is_empty(), |d| {
                d.child(
                    div()
                        .mt_2()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_size(px(self.settings.body_font_size))
                        .child(post.title.clone()),
                )
            })
            .child(
                div()
                    .my_3()
                    .text_size(px(self.settings.body_font_size))
                    .child(self.rich_content(("post-content", id), &post.content, cx)),
            )
            .when_some(post.image_url(true), |d, thumbnail| {
                let original = post.image_url(false).unwrap();
                d.child(
                    div()
                        .id(("post-image", id))
                        .my_2()
                        .cursor_pointer()
                        .child(media_view::tile(&self.media, thumbnail, px(220.), cx))
                        .child(
                            div()
                                .mt_1()
                                .text_xs()
                                .text_color(rgb(p.muted))
                                .child("点击查看原图"),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.viewing_image = Some(preview::ImageViewer::new(original.clone()));
                            cx.notify();
                        })),
                )
            })
            .when(!missing.is_empty(), |d| {
                d.child(
                    div()
                        .mb_2()
                        .text_xs()
                        .text_color(rgb(p.muted))
                        .child(format!(
                            "未加载 / 串外引用：{}",
                            missing
                                .iter()
                                .map(|id| format!("No.{id}"))
                                .collect::<Vec<_>>()
                                .join(" · ")
                        )),
                )
            })
            .child(
                div()
                    .id(("post-actions", id))
                    .flex()
                    .items_center()
                    .overflow_x_scroll()
                    .whitespace_nowrap()
                    .gap_3()
                    .when_some(post.image_url(false), |d, url| {
                        d.child(self.image_actions(("post-image-actions", id), url, cx))
                    })
                    .child(self.copy_text_button(("copy-post", id), &post.content, cx))
                    .child(button(p, ("quote", id), "引用回复").on_click(cx.listener(
                        move |this, _, window, cx| {
                            this.compose(this.selected, Some(id), window, cx)
                        },
                    )))
                    .when(children && self.aggregate, |d| {
                        d.child(
                            button(
                                p,
                                ("fold", id),
                                if collapsed {
                                    "展开分支"
                                } else {
                                    "收起分支"
                                },
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if !this.collapsed.remove(&id) {
                                        this.collapsed.insert(id);
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                    }),
            )
            .into_any_element()
    }

    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_panel.is_some() || self.sending {
            return;
        }
        let panel = cx.new(|cx| SettingsPanel::new(self.settings.clone(), window, cx));
        cx.subscribe_in(&panel, window, |this, panel, event, _, cx| {
            match event {
                SettingsEvent::Save(settings) if !this.settings_saving => {
                    this.settings_saving = true;
                    panel.update(cx, |panel, cx| panel.set_saving(true, cx));
                    let mut settings = settings.clone();
                    settings.pane_layout = this.settings.pane_layout.clone();
                    if this.tx.send(Request::Configure(settings)).is_err() {
                        this.settings_saving = false;
                        panel.update(cx, |panel, cx| {
                            panel.set_error("后台服务不可用，请重启客户端", cx)
                        });
                    }
                }
                SettingsEvent::Close if !this.settings_saving => this.settings_panel = None,
                _ => {}
            }
            cx.notify();
        })
        .detach();
        self.settings_panel = Some(panel);
        cx.notify();
    }

    fn header(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let p = Palette::current(cx);
        // GPUI's decoration query is Linux-specific; Windows returns Server even
        // when appears_transparent hides the native title bar.
        let client_decorated = cfg!(target_os = "windows")
            || matches!(window.window_decorations(), Decorations::Client { .. });
        let control = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .w(px(32.))
                .h(px(32.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .text_lg()
                .bg(rgb(p.sidebar))
                .hover(move |d| d.bg(rgb(p.hover)))
                // Native Windows caption handling must receive both events.
                .when(!cfg!(target_os = "windows"), |d| {
                    d.on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                })
                .child(label)
        };
        div()
            .id("client-titlebar")
            .h(px(48.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .px_3()
            .bg(rgb(p.surface))
            .border_b_1()
            .border_color(rgb(p.border))
            .when(!cfg!(target_os = "windows"), |d| {
                d.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, _| this.title_drag = true),
                )
                .on_mouse_down_out(cx.listener(|this, _, _, _| this.title_drag = false))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseUpEvent, window, cx| {
                        this.title_drag = false;
                        if event.click_count == 2 {
                            window.zoom_window();
                            cx.notify();
                        }
                    }),
                )
                .on_mouse_move(cx.listener(|this, _, window, _| {
                    if this.title_drag {
                        this.title_drag = false;
                        window.start_window_move();
                    }
                }))
                .on_mouse_down(MouseButton::Right, |event, window, _| {
                    window.show_window_menu(event.position)
                })
            })
            .child(
                div()
                    .w(px(240.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        button(p, "settings", "设置")
                            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                            })
                            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.open_settings(window, cx);
                            })),
                    )
                    .child(
                        button(p, "appearance", self.appearance.label())
                            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                window.prevent_default();
                                cx.stop_propagation();
                            })
                            .on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, window, cx| {
                                cx.stop_propagation();
                                this.appearance = this.appearance.next();
                                this.appearance.apply(window, cx);
                                apply_fonts(&this.settings, cx);
                                if this.appearance.save().is_err() {
                                    this.notice = "外观已切换，但无法保存偏好设置".into();
                                }
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    // GPUI 0.2.2 picks the first matching control hitbox, so a
                    // Drag area on the header would mask its caption buttons.
                    .when(cfg!(target_os = "windows"), |d| {
                        d.window_control_area(WindowControlArea::Drag)
                    })
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(APP_NAME),
            )
            .child(
                div()
                    .w(px(200.))
                    .when(!client_decorated, |d| d.invisible())
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        control("window-minimize", "−")
                            .when(cfg!(target_os = "windows"), |d| {
                                d.window_control_area(WindowControlArea::Min)
                            })
                            .when(!cfg!(target_os = "windows"), |d| {
                                d.on_click(|_, window, cx| {
                                    cx.stop_propagation();
                                    window.minimize_window();
                                })
                            }),
                    )
                    .child(
                        control(
                            "window-maximize",
                            if window.is_maximized() { "▣" } else { "□" },
                        )
                        .when(cfg!(target_os = "windows"), |d| {
                            d.window_control_area(WindowControlArea::Max)
                        })
                        .when(!cfg!(target_os = "windows"), |d| {
                            d.on_click(|_, window, cx| {
                                cx.stop_propagation();
                                window.zoom_window();
                            })
                        }),
                    )
                    .child(
                        control("window-close", "×")
                            .when(cfg!(target_os = "windows"), |d| {
                                d.window_control_area(WindowControlArea::Close)
                            })
                            .when(!cfg!(target_os = "windows"), |d| {
                                d.on_click(|_, window, cx| {
                                    cx.stop_propagation();
                                    window.remove_window();
                                })
                            }),
                    ),
            )
            .into_any_element()
    }

    fn page_toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = Palette::current(cx);
        let loading = self.navigation.loading();
        let current = self
            .reading_restore
            .as_ref()
            .map(|position| position.page)
            .or_else(|| {
                self.capture_reading_position()
                    .map(|position| position.page)
            })
            .unwrap_or(self.navigation.current());
        let total = self.navigation.total();
        let range = self.navigation.range();
        let before = self.navigation.before();
        let after = self.navigation.after();
        div()
            .flex()
            .flex_col()
            .min_h_0()
            .flex_shrink_0()
            .px_4()
            .pb_2()
            .border_b_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .flex()
                    .id("page-navigation-row")
                    .overflow_x_scroll()
                    .w_full()
                    .min_w_0()
                    .min_h_0()
                    .whitespace_nowrap()
                    .flex_shrink_0()
                    .items_center()
                    .content_start()
                    .gap_2()
                    .child(
                        button(p, "previous-page", "上一页")
                            .when(loading || self.navigation.before().is_none(), |d| {
                                d.opacity(0.4)
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(page) = this.navigation.before() {
                                    this.request_page(page, PageMode::Buffer, cx);
                                }
                            })),
                    )
                    .child(div().flex_shrink_0().text_xs().child(if current == 0 {
                        "— / — 页".to_string()
                    } else {
                        format!("{current} / {total} 页")
                    }))
                    .child(
                        button(p, "next-page", "下一页")
                            .when(loading || self.navigation.after().is_none(), |d| {
                                d.opacity(0.4)
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(page) = this.navigation.after() {
                                    this.request_page(page, PageMode::Buffer, cx);
                                }
                            })),
                    )
                    .child(
                        div()
                            .w(px(72.))
                            .flex_shrink_0()
                            .child(Input::new(&self.page_input).disabled(loading || current == 0)),
                    )
                    .child(
                        button(p, "go-page", "跳转")
                            .when(loading || current == 0, |d| d.opacity(0.4))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.navigation.current() > 0 {
                                    this.jump_to_page(cx);
                                }
                            })),
                    )
                    .when(loading, |d| {
                        d.child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(rgb(p.muted))
                                .child("加载中…"),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .id("page-buffer-row")
                    .overflow_x_scroll()
                    .w_full()
                    .min_w_0()
                    .min_h_0()
                    .whitespace_nowrap()
                    .flex_shrink_0()
                    .items_center()
                    .content_start()
                    .gap_2()
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(rgb(p.muted))
                            .child(
                                range
                                    .map(|(first, last)| {
                                        format!("已加载 {first}–{last} 页 · 跳转后从目标页重新聚合")
                                    })
                                    .unwrap_or_else(|| "尚未加载页面".into()),
                            ),
                    )
                    .when(self.aggregate, |d| {
                        d.child(
                            button(p, "buffer-before", "向前缓冲 1 页")
                                .when(loading || before.is_none(), |d| d.opacity(0.4))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(page) = this.navigation.before() {
                                        this.request_page(page, PageMode::Buffer, cx);
                                    }
                                })),
                        )
                        .child(
                            button(p, "buffer-after", "向后缓冲 1 页")
                                .when(loading || after.is_none(), |d| d.opacity(0.4))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(page) = this.navigation.after() {
                                        this.request_page(page, PageMode::Buffer, cx);
                                    }
                                })),
                        )
                    }),
            )
            .into_any_element()
    }

    fn rows(&self) -> Vec<(u64, usize)> {
        let Some(tree) = &self.tree else {
            return vec![];
        };
        if !self.aggregate {
            return self
                .navigation
                .linear_ids()
                .into_iter()
                .map(|id| (id, 0))
                .collect();
        }
        let mut rows = vec![];
        let mut stack: Vec<_> = tree
            .nodes
            .iter()
            .filter(|(_, n)| n.parent.is_none())
            .map(|(id, _)| (*id, 0))
            .rev()
            .collect();
        let mut seen = HashSet::new();
        while let Some((id, depth)) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            rows.push((id, depth));
            if !self.collapsed.contains(&id)
                && let Some(node) = tree.nodes.get(&id)
            {
                stack.extend(node.children.iter().rev().map(|id| (*id, depth + 1)));
            }
        }
        rows
    }
}

fn apply_fonts(settings: &Settings, cx: &mut App) {
    let theme = gpui_component::Theme::global_mut(cx);
    theme.font_family = settings.font_family.clone().into();
    theme.font_size = px(settings.ui_font_size);
}

fn button(p: Palette, id: impl Into<ElementId>, label: impl Into<SharedString>) -> Stateful<Div> {
    div()
        .id(id)
        .flex_shrink_0()
        .px_3()
        .py_2()
        .rounded_md()
        .text_xs()
        .text_color(rgb(p.accent))
        .cursor_pointer()
        .hover(move |d| d.bg(rgb(p.hover)))
        .child(label.into())
}
fn heading(p: Palette, kicker: &str, title: String) -> Div {
    div()
        .p_5()
        .border_b_1()
        .border_color(rgb(p.border))
        .child(
            div()
                .text_xs()
                .text_color(rgb(p.muted))
                .child(kicker.to_string()),
        )
        .child(
            div()
                .mt_2()
                .text_lg()
                .font_weight(FontWeight::SEMIBOLD)
                .child(title),
        )
}

impl Render for Island {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.install_preview_shortcuts(window, cx);
        let p = Palette::current(cx);
        self.clear_current_draft(window, cx);
        self.schedule_reading_restore(window, cx);
        self.schedule_cached_short_check(window, cx);
        let mut forums = div()
            .id("forums-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px_3();
        forums = self.timeline_sidebar(forums, p, cx);
        for group in &self.groups {
            forums = forums.child(
                div()
                    .px_3()
                    .pt_5()
                    .pb_2()
                    .text_xs()
                    .text_color(rgb(p.muted))
                    .child(group.name.clone()),
            );
            for forum in &group.forums {
                if forum.id < 0 {
                    continue;
                }
                let id = forum.id;
                let name = forum.display_name();
                forums =
                    forums.child(
                        button(p, ("forum", id as u64), name.clone())
                            .w_full()
                            .mb_1()
                            .when(
                                !self.showing_history
                                    && !self.showing_subscriptions
                                    && self.timeline.is_none()
                                    && self.forum == id,
                                |d| d.bg(rgb(p.selected)).font_weight(FontWeight::SEMIBOLD),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.load_forum(id, name.clone(), cx)
                            })),
                    );
            }
        }
        let sidebar = div()
            .w_full()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .bg(rgb(p.sidebar))
            .child(heading(p, "R D N M B", APP_NAME.into()))
            .child(
                button(p, "subscriptions", "☆ 我的订阅")
                    .mx_3()
                    .mb_2()
                    .when(self.showing_subscriptions && !self.showing_history, |d| {
                        d.bg(rgb(p.selected))
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        if !this.showing_subscriptions || !this.forum_loading {
                            this.load_subscriptions(cx);
                        }
                    })),
            )
            .child(
                button(p, "history", "◷ 历史记录")
                    .mx_3()
                    .mb_2()
                    .when(self.showing_history, |d| d.bg(rgb(p.selected)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.open_history(HistoryCategory::Browsing, cx)
                    })),
            )
            .child(forums)
            .child(
                button(p, "reconnect", "重新获取板块").on_click(cx.listener(|this, _, _, _| {
                    if !this.groups_loading {
                        this.groups_loading = true;
                        this.tx.send(Request::Forums).ok();
                    }
                })),
            )
            .child(
                div()
                    .p_4()
                    .text_xs()
                    .text_color(rgb(p.muted))
                    .child(if self.cookie {
                        "● 本地饼干已载入"
                    } else {
                        "○ 只读浏览"
                    }),
            );
        let mut list = div()
            .id(("threads-scroll", self.forum_epoch))
            .track_scroll(&self.forum_scroll)
            .on_scroll_wheel(cx.listener(Self::scroll_forum))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3();
        for post in &self.threads {
            let id = post.id;
            let subscription_view = cx.entity().downgrade();
            let preview: String = post
                .text()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(100)
                .collect();
            // ContextMenuExt uses a fixed element ID. Scope the wrapper itself
            // per thread, otherwise every row shares and paints the same menu.
            list = list.child(
                div().id(("thread-context", id)).child(
                    div()
                        .id(("thread", id))
                        .p_4()
                        .mb_2()
                        .rounded_lg()
                        .cursor_pointer()
                        .bg(rgb(if self.selected == Some(id) {
                            p.selected
                        } else {
                            p.surface
                        }))
                        .hover(move |d| d.bg(rgb(p.hover)))
                        .child(
                            div()
                                .text_xs()
                                .text_color(rgb(p.muted))
                                .child(format!("No.{}  ·  {} 回复", id, post.reply_count)),
                        )
                        .child(
                            div()
                                .mt_2()
                                .text_size(px(self.settings.thread_font_size))
                                .child(preview),
                        )
                        .when_some(post.image_url(true), |d, url| {
                            d.child(div().mt_2().child(media_view::tile(
                                &self.media,
                                url,
                                px(110.),
                                cx,
                            )))
                        })
                        .when(self.showing_subscriptions, |d| {
                            d.child(
                                button(
                                    p,
                                    ("remove-subscription", id),
                                    if self.subscription_pending.contains(&id) {
                                        "处理中…"
                                    } else {
                                        "移除订阅"
                                    },
                                )
                                .mt_2()
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.change_subscription(id, false, cx);
                                    },
                                )),
                            )
                        })
                        .on_click(cx.listener(move |this, _, _, cx| this.open_thread(id, cx)))
                        .context_menu(move |menu, _, cx| {
                            let Some(view) = subscription_view.upgrade() else {
                                return menu;
                            };
                            let state = view.read(cx);
                            let add = !state.subscribed.contains(&id);
                            let pending = state.subscription_pending.contains(&id);
                            menu.item(
                                PopupMenuItem::new(if add {
                                    "添加订阅"
                                } else {
                                    "移除订阅"
                                })
                                .disabled(pending)
                                .on_click(move |_, _, cx| {
                                    view.update(cx, |this, cx| {
                                        this.change_subscription(id, add, cx)
                                    })
                                }),
                            )
                        }),
                ),
            );
        }
        list = list.child(
            div()
                .py_3()
                .text_xs()
                .text_center()
                .text_color(rgb(p.muted))
                .child(if self.forum_loading {
                    "正在加载…"
                } else if self.forum_end {
                    if self.showing_subscriptions {
                        "已到订阅列表末尾"
                    } else if self.timeline.is_some() {
                        "已到时间线末尾"
                    } else {
                        "已到板块末尾"
                    }
                } else if self.forum_paging.failed {
                    "加载失败，当前内容已保留"
                } else {
                    "滑到底部自动加载下一页"
                })
                .when(
                    !self.forum_loading
                        && !self.timeline_limit_reached(self.forum_page)
                        && (self.forum_end
                            || self.forum_paging.failed
                            || self.forum_scroll.max_offset().height <= px(2.)),
                    |d| {
                        d.child(
                            button(
                                p,
                                "more-threads",
                                if self.forum_end {
                                    "重新检查下一页"
                                } else if self.forum_paging.failed {
                                    "重试加载"
                                } else {
                                    "手动加载下一页"
                                },
                            )
                            .mt_2()
                            .w_full()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.forum_paging.retry();
                                this.forum_end = false;
                                this.request_forum_next(cx);
                            })),
                        )
                    },
                ),
        );
        let thread_list = div()
            .w_full()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .border_r_1()
            .border_color(rgb(p.border))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(rgb(p.border))
                    .child(div().flex_1().min_w_0().child(heading(
                        p,
                        if self.showing_subscriptions {
                            "FEED / 订阅"
                        } else if self.timeline.is_some() {
                            "TIMELINE / 时间线"
                        } else {
                            "BOARD / 板块"
                        },
                        if self.showing_subscriptions {
                            "我的订阅".into()
                        } else if let Some(timeline) = &self.timeline {
                            timeline.display_name()
                        } else {
                            self.forum_name.clone()
                        },
                    )))
                    .child(
                        button(
                            p,
                            "refresh-forum",
                            if self.forum_loading {
                                "加载中…"
                            } else {
                                "刷新"
                            },
                        )
                        .mr_2()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if !this.forum_loading {
                                if this.showing_subscriptions {
                                    this.load_subscriptions(cx);
                                } else if let Some(timeline) = this.timeline.clone() {
                                    this.load_timeline(timeline, cx);
                                } else {
                                    this.load_forum(this.forum, this.forum_name.clone(), cx);
                                }
                            }
                        })),
                    ),
            )
            .child(
                button(
                    p,
                    "new-thread",
                    if self.showing_subscriptions {
                        "＋ 发串到当前板块"
                    } else {
                        "＋ 发起新串"
                    },
                )
                .mx_3()
                .mt_2()
                .on_click(cx.listener(|this, _, window, cx| this.compose(None, None, window, cx))),
            )
            .child(list);
        let mut reading = div()
            .id(("reading-scroll", self.reading_revision))
            .track_scroll(&self.reading_scroll)
            .on_scroll_wheel(cx.listener(Self::scroll_reading))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_5();
        if self.selected.is_none() {
            reading = reading.child(
                div()
                    .mt_20()
                    .text_center()
                    .text_color(rgb(p.muted))
                    .child("在左侧选一篇串\n把喧嚣留在外面，慢慢读。"),
            );
        }
        for (id, depth) in self.rows() {
            if let Some(node) = self.tree.as_ref().and_then(|tree| tree.nodes.get(&id)) {
                reading = reading.child(self.post_card(
                    &node.post,
                    depth,
                    &node.missing_references,
                    !node.children.is_empty(),
                    cx,
                ));
            }
        }
        if self.selected.is_some() {
            let end = self.reading_next().is_none();
            reading = reading.child(
                div()
                    .py_3()
                    .text_xs()
                    .text_center()
                    .text_color(rgb(p.muted))
                    .child(if self.navigation.loading() {
                        "正在加载…"
                    } else if self.reading_paging.failed {
                        if self.tree.is_some() {
                            "加载失败，当前内容已保留"
                        } else {
                            "加载失败，请重试"
                        }
                    } else if end {
                        "已到当前最后一页"
                    } else {
                        "滑到底部自动加载下一页"
                    })
                    .when(
                        !self.navigation.loading()
                            && (end
                                || self.reading_paging.failed
                                || self.reading_scroll.max_offset().height <= px(2.)),
                        |d| {
                            d.child(
                                button(
                                    p,
                                    "more-replies",
                                    if self.reading_paging.failed {
                                        "重试加载"
                                    } else if end {
                                        "刷新，检查新回复"
                                    } else {
                                        "手动加载下一页"
                                    },
                                )
                                .mt_2()
                                .w_full()
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.reading_paging.retry();
                                        let request =
                                            this.reading_retry.or_else(|| this.reading_next());
                                        if let Some((page, mode)) = request {
                                            this.request_page(page, mode, cx);
                                        } else {
                                            let page = this
                                                .navigation
                                                .range()
                                                .map(|(_, last)| last)
                                                .unwrap_or(1);
                                            this.request_page(page, PageMode::Recheck, cx);
                                        }
                                    },
                                )),
                            )
                        },
                    ),
            );
        }
        let reader = div()
            .w_full()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(heading(
                p,
                "THREAD / 阅读",
                self.selected
                    .map(|id| format!("No.{id}"))
                    .unwrap_or("此刻，岛上".into()),
            ))
            .child(
                div()
                    .flex()
                    .id("reader-actions-row")
                    .overflow_x_scroll()
                    .w_full()
                    .min_w_0()
                    .min_h_0()
                    // Keep toolbar height independent of wrapped flex intrinsic measurements.
                    .whitespace_nowrap()
                    .flex_shrink_0()
                    .items_center()
                    .content_start()
                    .px_4()
                    .py_2()
                    .gap_2()
                    .child(
                        button(p, "chronological", "时间顺序")
                            .when(!self.aggregate, |d| d.bg(rgb(p.selected)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.aggregate {
                                    this.aggregate = false;
                                    this.reset_reading_scroll();
                                }
                                cx.notify();
                            })),
                    )
                    .when(self.selected.is_some(), |d| {
                        d.child(button(p, "refresh-thread", "刷新").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.request_page(1, PageMode::Refresh, cx);
                            },
                        )))
                    })
                    .when(self.selected.is_some(), |d| {
                        d.child(
                            button(p, "clear-thread-cache", "清除本串缓存并重载").on_click(
                                cx.listener(|this, _, _, cx| this.clear_thread_cache(cx)),
                            ),
                        )
                    })
                    .child(
                        button(p, "aggregate", "引用树")
                            .when(self.aggregate, |d| d.bg(rgb(p.selected)))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !this.aggregate {
                                    this.aggregate = true;
                                    this.reset_reading_scroll();
                                }
                                cx.notify();
                            })),
                    )
                    .when_some(self.selected, |d, id| {
                        d.child(self.subscription_button(id, cx))
                    })
                    .when(self.selected.is_some(), |d| {
                        d.child(button(p, "reply", "＋ 回复").on_click(cx.listener(
                            |this, _, window, cx| this.compose(this.selected, None, window, cx),
                        )))
                    }),
            )
            .when(self.selected.is_some(), |d| d.child(self.page_toolbar(cx)))
            .child(reading)
            .with_animation(
                ("reader-enter", self.thread_epoch),
                Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint()),
                |d, delta| d.opacity(0.3 + 0.7 * delta),
            );
        let editor = if self.composing {
            let editor = div()
                .capture_action(
                    cx.listener(|this, _: &gpui_component::input::Paste, _, cx| {
                        if this.paste_image(false, cx) {
                            cx.stop_propagation();
                        }
                    }),
                )
                .w_full()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .bg(rgb(p.surface))
                .border_l_1()
                .border_color(rgb(p.border))
                .child(heading(p, "WRITE / 写点什么", self.compose_label.clone()))
                .child(div().px_4().child(self.cookie_dropdown(cx)))
                .child(
                    div()
                        .p_4()
                        .child(Input::new(&self.title).disabled(self.sending)),
                )
                .child(
                    div().px_4().flex_1().min_h_0().child(
                        Input::new(&self.body)
                            .disabled(self.sending)
                            .text_size(px(self.settings.body_font_size))
                            .h_full(),
                    ),
                )
                .when(self.kaomoji_open, |d| d.child(self.kaomoji_picker(cx)))
                .child(
                    div()
                        .px_4()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .child(
                            button(
                                p,
                                "toggle-kaomoji",
                                if self.kaomoji_open {
                                    "收起颜文字"
                                } else {
                                    "颜文字"
                                },
                            )
                            .when(self.sending, |d| d.opacity(0.4))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !this.sending {
                                    this.kaomoji_open = !this.kaomoji_open;
                                    cx.notify();
                                }
                            })),
                        )
                        .child(
                            button(
                                p,
                                "choose-image",
                                if self.attachment_loading {
                                    "正在读取…"
                                } else {
                                    "选择图片"
                                },
                            )
                            .when(self.sending || self.attachment_loading, |d| d.opacity(0.4))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.choose_image(window, cx)),
                            ),
                        )
                        .child(
                            button(p, "paste-image", "粘贴图片")
                                .when(self.sending || self.attachment_loading, |d| d.opacity(0.4))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.paste_image(true, cx);
                                })),
                        ),
                )
                .when_some(self.attachment.as_ref(), |d, image| {
                    d.child(
                        div()
                            .px_4()
                            .py_2()
                            .child(
                                div()
                                    .relative()
                                    .w_full()
                                    .h(px(96.))
                                    .flex_shrink_0()
                                    .overflow_hidden()
                                    .child(
                                        img(image.preview.clone())
                                            .absolute()
                                            .inset_0()
                                            .size_full()
                                            .object_fit(ObjectFit::Contain),
                                    ),
                            )
                            .child(div().text_xs().text_color(rgb(p.muted)).child(format!(
                                "{} · {:.1} KiB",
                                image.attachment.filename(),
                                image.attachment.size() as f64 / 1024.
                            )))
                            .child(button(p, "remove-attachment", "移除图片").on_click(
                                cx.listener(|this, _, _, cx| {
                                    if !this.sending {
                                        this.attachment = None;
                                        this.attachment_revision += 1;
                                        this.attachment_loading = false;
                                        cx.notify();
                                    }
                                }),
                            )),
                    )
                })
                .child(
                    button(p, "clear-draft", "清空草稿")
                        .mx_4()
                        .on_click(cx.listener(|this, _, _, cx| {
                            if !this.sending {
                                this.clear_draft = true;
                                cx.notify();
                            }
                        })),
                )
                .child(
                    div()
                        .p_4()
                        .text_xs()
                        .text_color(rgb(p.muted))
                        .child("友善交流，认真生活。\n发送前请检查目标与内容。"),
                )
                .child(
                    div()
                        .flex()
                        .p_4()
                        .justify_between()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            button(p, "close-editor", "收起 / 保留草稿").on_click(cx.listener(
                                |this, _, _, cx| {
                                    if !this.sending {
                                        this.composing = false;
                                        this.kaomoji_open = false;
                                        cx.notify();
                                    }
                                },
                            )),
                        )
                        .child(
                            button(
                                p,
                                "send",
                                if self.sending {
                                    "发送中…"
                                } else if self.posting_cookie_name().is_none() {
                                    "需要饼干"
                                } else {
                                    "发送 ↗"
                                },
                            )
                            .bg(rgb(p.selected))
                            .on_click(cx.listener(|this, _, _, cx| this.send(cx))),
                        ),
                )
                .with_animation(
                    ("editor-enter", self.animation),
                    Animation::new(Duration::from_millis(240)).with_easing(ease_out_quint()),
                    |d, delta| d.opacity(delta),
                );
            Some(editor.into_any_element())
        } else {
            None
        };
        let reader = if self.showing_history {
            self.render_history(cx)
        } else {
            reader.into_any_element()
        };
        let columns = self.panes.render(
            sidebar.into_any_element(),
            thread_list.into_any_element(),
            reader,
            editor,
            window,
            cx,
        );
        div()
            .capture_any_mouse_down(cx.listener(Self::preview_mouse_down))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(p.background))
            .text_color(rgb(p.foreground))
            .font_family(self.settings.font_family.clone())
            .text_size(px(self.settings.ui_font_size))
            .child(self.header(window, cx))
            .child(if let Some(panel) = &self.settings_panel {
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .p_4()
                    .child(panel.clone())
                    .into_any_element()
            } else if self.viewing_image.is_some() {
                self.image_overlay(cx)
            } else {
                columns.into_any_element()
            })
            .child(
                div()
                    .h(px(32.))
                    .flex_shrink_0()
                    .px_4()
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(rgb(p.muted))
                    .border_t_1()
                    .border_color(rgb(p.border))
                    .child(self.notice.clone()),
            )
            .relative()
            .when(!self.previews.is_empty(), |d| {
                d.child(self.preview_overlay(cx))
            })
    }
}

fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();
    let cookie = ["RDNMB_COOKIE", "NMBXD_COOKIE", "userhash", "usrhash"]
        .iter()
        .find_map(|key| std::env::var(key).ok())
        .filter(|s| !s.trim().is_empty());
    let settings = Settings::load_or_initialize(cookie.unwrap_or_default())?;
    let api = ApiClient::with_website_base(Some(&settings.cookie), &settings.website_base)?;
    if std::env::args().any(|arg| arg == "--smoke") {
        let groups = api.forum_groups()?;
        std::thread::sleep(Duration::from_millis(800));
        let posts = api.forum_page(4, 1)?;
        println!(
            "Read-only API smoke: {} forum groups, {} threads",
            groups.len(),
            posts.len()
        );
        if let Some(post) = posts.first() {
            std::thread::sleep(Duration::from_millis(800));
            let thread = api.thread_page(post.id, 1)?;
            println!("Thread page: {} replies", thread.replies.len());
        }
        return Ok(());
    }
    Application::new()
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            cx.on_app_quit(|cx| {
                cx.background_executor()
                    .spawn(async { image_clipboard::shutdown() })
            })
            .detach();
            emoji::init(cx).expect("Unable to load bundled emoji font");
            gpui_component::init(cx);
            cx.on_window_closed(|cx| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(1400.), px(900.)),
                        cx,
                    ))),
                    app_id: Some(APP_ID.into()),
                    window_decorations: Some(WindowDecorations::Client),
                    window_background: WindowBackgroundAppearance::Transparent,
                    window_min_size: Some(size(px(1100.), px(640.))),
                    titlebar: Some(TitlebarOptions {
                        title: Some(APP_NAME.into()),
                        appears_transparent: true,
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    let view = cx.new(|cx| Island::new(window, cx, api, settings));
                    cx.new(|cx| Root::new(view, window, cx))
                },
            )
            .expect("Unable to create window");
            cx.activate(true);
        });
    Ok(())
}
