//! Blocking HTTP client. Call from worker threads, never from the GUI update loop.
use crate::model::{ForumGroup, Post, html_to_text};
use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use reqwest::blocking::{Client, RequestBuilder, multipart};
use reqwest::header::{COOKIE, HeaderMap, HeaderValue, LOCATION};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{
    fmt,
    io::Cursor,
    sync::{Arc, OnceLock},
    time::Duration,
};

pub const API_BASE: &str = "https://api.nmb.best/api/";
pub const WEBSITE_BASE: &str = "https://www.nmbxd1.com/";

/// Client-side upload cap; the server may impose a smaller limit.
pub const MAX_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// The service accepts arbitrary IDs; this client bounds settings and request size.
pub const MAX_SUBSCRIPTION_ID_CHARS: usize = 256;

pub fn validate_subscription_id(value: &str) -> Result<&str> {
    let value = value.trim();
    if value.is_empty() {
        bail!("请先在设置中填写订阅 ID");
    }
    if value.chars().count() > MAX_SUBSCRIPTION_ID_CHARS {
        bail!("订阅 ID 不能超过 {MAX_SUBSCRIPTION_ID_CHARS} 个字符");
    }
    Ok(value)
}

/// A reference may name a thread or one of its replies. The reference endpoint
/// does not expose the enclosing thread ID; callers may supply it from known
/// thread context, but must not assume that `post.id` is a thread ID.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ReferencePreview {
    #[serde(flatten)]
    pub post: Post,
    #[serde(skip)]
    pub thread_id: Option<u64>,
}

/// A validated, immutable attachment shared by the draft and sending worker.
#[derive(Clone)]
pub struct ImageAttachment {
    filename: String,
    bytes: Arc<[u8]>,
    mime_type: &'static str,
}

impl ImageAttachment {
    pub fn new(filename: impl AsRef<str>, bytes: Vec<u8>) -> Result<Self> {
        if bytes.is_empty() {
            bail!("图片文件为空");
        }
        if bytes.len() > MAX_IMAGE_BYTES {
            bail!("图片不能超过 10 MiB，请缩小后重试");
        }
        let format =
            image::guess_format(&bytes).context("无法识别图片，请选择 JPEG、PNG 或 GIF")?;
        let (extension, mime_type) = match format {
            image::ImageFormat::Jpeg => ("jpg", "image/jpeg"),
            image::ImageFormat::Png => ("png", "image/png"),
            image::ImageFormat::Gif => ("gif", "image/gif"),
            _ => bail!("仅支持 JPEG、PNG 和 GIF 图片"),
        };
        // Bound decoding before accepting compressed files with enormous dimensions.
        // Keep original bytes (including GIF animation) for the eventual upload.
        let mut reader = image::ImageReader::with_format(Cursor::new(&bytes), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(16_384);
        limits.max_image_height = Some(16_384);
        limits.max_alloc = Some(128 * 1024 * 1024);
        reader.limits(limits);
        reader.decode().context("图片损坏或尺寸过大，请另选图片")?;

        let basename = filename.as_ref().rsplit(['/', '\\']).next().unwrap_or("");
        let stem = basename.rsplit_once('.').map_or(basename, |(stem, _)| stem);
        let stem: String = stem
            .chars()
            .filter(|ch| ch.is_alphanumeric() || matches!(ch, '-' | '_' | '.' | ' '))
            .take(80)
            .collect();
        let stem = stem.trim_matches(['.', ' ']);
        let stem = if stem.is_empty() { "attachment" } else { stem };
        Ok(Self {
            filename: format!("{stem}.{extension}"),
            bytes: bytes.into(),
            mime_type,
        })
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn mime_type(&self) -> &'static str {
        self.mime_type
    }

    pub fn size(&self) -> usize {
        self.bytes.len()
    }
}

impl fmt::Debug for ImageAttachment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ImageAttachment")
            .field("filename", &self.filename)
            .field("size", &self.size())
            .field("mime_type", &self.mime_type)
            .finish()
    }
}

/// Website origin used only for posting; JSON reads retain their separate API host.
pub fn normalize_website_base(value: &str) -> Result<String> {
    let value = value.trim();
    let value = if value.is_empty() {
        WEBSITE_BASE
    } else {
        value
    };
    let url = if value.contains("://") {
        value.to_owned()
    } else {
        format!("https://{value}")
    };
    let url = reqwest::Url::parse(&url).map_err(|_| anyhow!("网页地址无效，请填写 HTTPS 域名"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        bail!("网页地址须为 HTTPS 站点根地址，不含用户名、路径、查询参数或片段");
    }
    Ok(url.to_string())
}

#[derive(Clone)]
pub struct ApiClient {
    client: Client,
    has_cookie: bool,
    website_base: String,
}

// Do not let diagnostic output expose the client's authentication headers.
impl fmt::Debug for ApiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiClient")
            .field("has_cookie", &self.has_cookie)
            .finish_non_exhaustive()
    }
}

impl ApiClient {
    /// Supply the environment's userhash value, or a Cookie string containing
    /// `userhash=…`. Only that cookie is used; credentials are never persisted.
    pub fn new(cookie: Option<&str>) -> Result<Self> {
        Self::with_website_base(cookie, WEBSITE_BASE)
    }

    pub fn with_website_base(cookie: Option<&str>, website_base: &str) -> Result<Self> {
        let website_base = normalize_website_base(website_base)?;
        let cookie = cookie.map(str::trim).filter(|s| !s.is_empty());
        let mut headers = HeaderMap::new();
        let mut has_cookie = false;
        if let Some(cookie) = cookie {
            let value = cookie
                .split(';')
                .find_map(|part| part.trim().strip_prefix("userhash="))
                .unwrap_or(cookie)
                .trim();
            if value.is_empty() || value.contains(';') {
                bail!("饼干格式无效：请提供 userhash 值或含 userhash 的 Cookie");
            }
            let mut header = HeaderValue::from_str(&format!("userhash={value}"))
                .map_err(|_| anyhow!("饼干包含无效的 HTTP 头字符"))?;
            header.set_sensitive(true);
            headers.insert(COOKIE, header);
            has_cookie = true;
        }
        let client = Client::builder()
            .default_headers(headers)
            .user_agent("nmbxd-rs/0.1")
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("无法初始化网络客户端")?;
        Ok(Self {
            client,
            has_cookie,
            website_base,
        })
    }

    pub fn website_base(&self) -> &str {
        &self.website_base
    }

    pub fn has_cookie(&self) -> bool {
        self.has_cookie
    }

    pub fn forum_groups(&self) -> Result<Vec<ForumGroup>> {
        self.read(self.client.get(format!("{API_BASE}getForumList")))
    }

    pub fn forum_page(&self, id: i64, page: u32) -> Result<Vec<Post>> {
        if id <= 0 {
            bail!("请选择有效版面（时间线不作为版面请求）");
        }
        if page == 0 {
            bail!("页码必须大于零");
        }
        self.read(
            self.client
                .get(format!("{API_BASE}showf"))
                .query(&[("id", id.to_string()), ("page", page.to_string())]),
        )
    }

    pub fn thread_page(&self, id: u64, page: u32) -> Result<Post> {
        if id == 0 || page == 0 {
            bail!("串号和页码必须大于零");
        }
        self.read(
            self.client
                .get(format!("{API_BASE}thread"))
                .query(&[("id", id.to_string()), ("page", page.to_string())]),
        )
    }

    /// Server subscriptions belong to this ID, independently of the posting cookie.
    /// The API has no individual membership lookup; only returned feed entries are
    /// known to be subscribed. An absent entry on one page proves nothing.
    pub fn subscriptions(&self, uuid: &str, page: u32) -> Result<Vec<Post>> {
        self.subscriptions_at(&format!("{API_BASE}feed"), uuid, page)
    }

    fn subscriptions_at(&self, endpoint: &str, uuid: &str, page: u32) -> Result<Vec<Post>> {
        let uuid = validate_subscription_id(uuid)?;
        if page == 0 {
            bail!("页码必须大于零");
        }
        let value = self.subscription_json(
            self.client
                .get(endpoint)
                .query(&[("uuid", uuid.to_owned()), ("page", page.to_string())]),
        )?;
        // Do not retain serde or server error strings, which may echo this private ID.
        decode_read(value)
            .map_err(|_| anyhow!("无法读取订阅列表：服务器返回的数据无效或拒绝了请求"))
    }

    /// Send once. Repeated adds succeed too; success cannot establish prior membership.
    pub fn add_subscription(&self, uuid: &str, id: u64) -> Result<String> {
        self.change_subscription(&format!("{API_BASE}addFeed"), uuid, id, "订阅大成功→_→")
    }

    /// Removing an absent or deleted thread also succeeds on the server.
    pub fn remove_subscription(&self, uuid: &str, id: u64) -> Result<String> {
        self.change_subscription(&format!("{API_BASE}delFeed"), uuid, id, "取消订阅成功!")
    }

    fn change_subscription(
        &self,
        endpoint: &str,
        uuid: &str,
        id: u64,
        expected: &str,
    ) -> Result<String> {
        let uuid = validate_subscription_id(uuid)?;
        if id == 0 {
            bail!("串号必须大于零");
        }
        let value = self.subscription_json(
            self.client
                .post(endpoint)
                .query(&[("uuid", uuid)])
                .form(&[("tid", id.to_string())]),
        )?;
        parse_subscription_response(value, expected)
    }

    fn subscription_json(&self, request: RequestBuilder) -> Result<Value> {
        // reqwest errors include the URL (and therefore the subscription ID).
        // Deliberately discard their sources, including from chained diagnostics.
        let response = request
            .send()
            .map_err(|_| anyhow!("订阅请求结果未确认，请检查网络连接并刷新列表核实"))?;
        if !response.status().is_success() {
            bail!(
                "订阅请求返回 HTTP {}，请刷新订阅列表核实",
                response.status()
            );
        }
        response
            .json()
            .map_err(|_| anyhow!("服务器没有返回有效的订阅结果，请刷新列表核实"))
    }

    /// Load a single referenced post, including references to individual replies.
    /// `/ref` deliberately leaves `thread_id` unknown; `/thread` can separately
    /// verify a thread ID, while replies require enclosing thread context.
    pub fn reference(&self, id: u64) -> Result<ReferencePreview> {
        if id == 0 {
            bail!("引用串号必须大于零");
        }
        let preview: ReferencePreview = self.read(
            self.client
                .get(format!("{API_BASE}ref"))
                .query(&[("id", id.to_string())]),
        )?;
        if preview.post.id != id {
            bail!("服务器返回的引用串号与请求不符");
        }
        Ok(preview)
    }

    fn read<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T> {
        let response = request.send().context("读取失败，请检查网络连接")?;
        let status = response.status();
        if !status.is_success() {
            bail!("服务器返回 HTTP {status}");
        }
        let value: Value = response.json().context("服务器没有返回有效 JSON")?;
        decode_read(value)
    }

    /// Text-only posting. A call sends immediately and is never retried.
    pub fn post_thread(&self, fid: i64, name: &str, title: &str, content: &str) -> Result<String> {
        self.post_thread_with_image(fid, name, title, content, None)
    }

    /// Send once with an optional image using the website's `image` file field.
    pub fn post_thread_with_image(
        &self,
        fid: i64,
        name: &str,
        title: &str,
        content: &str,
        image: Option<&ImageAttachment>,
    ) -> Result<String> {
        if fid <= 0 {
            bail!("请选择有效版面");
        }
        self.post(
            "doPostThread.html",
            ("fid", fid.to_string()),
            name,
            title,
            content,
            image,
        )
    }

    /// Text-only reply. Quoting a specific reply is expressed in `content`
    /// (`>>No.123`); `thread_id` must remain the enclosing thread's ID.
    pub fn reply(&self, thread_id: u64, name: &str, title: &str, content: &str) -> Result<String> {
        self.reply_with_image(thread_id, name, title, content, None)
    }

    pub fn reply_with_image(
        &self,
        thread_id: u64,
        name: &str,
        title: &str,
        content: &str,
        image: Option<&ImageAttachment>,
    ) -> Result<String> {
        if thread_id == 0 {
            bail!("请选择有效主题串");
        }
        self.post(
            "doReplyThread.html",
            ("resto", thread_id.to_string()),
            name,
            title,
            content,
            image,
        )
    }

    fn post(
        &self,
        action: &str,
        target: (&str, String),
        name: &str,
        title: &str,
        content: &str,
        image: Option<&ImageAttachment>,
    ) -> Result<String> {
        if !self.has_cookie {
            bail!("发串或回复需要配置 userhash 饼干");
        }
        if content.trim().is_empty() && image.is_none() {
            bail!("正文和图片不能同时为空");
        }
        let form = [
            (target.0, target.1.as_str()),
            ("name", name),
            ("title", title),
            ("content", content),
        ];
        let request = self
            .client
            .post(format!("{}home/forum/{action}", self.website_base));
        let request = if let Some(image) = image {
            let part = multipart::Part::bytes(image.bytes().to_vec())
                .file_name(image.filename().to_owned())
                .mime_str(image.mime_type())?;
            let mut multipart = multipart::Form::new();
            for (key, value) in form {
                multipart = multipart.text(key.to_owned(), value.to_owned());
            }
            request.multipart(multipart.part("image", part))
        } else {
            request.form(&form)
        };
        let response = request
            .send()
            .context("发送结果未确认，请刷新原串或版面核实后再决定是否重试")?;
        if response.status().is_redirection() {
            // Do not replay POSTs or forward a credential to an unchosen origin.
            // Only display origin + path, never redirect query tokens or userinfo.
            let destination = response
                .headers()
                .get(LOCATION)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| response.url().join(value).ok())
                .filter(|url| matches!(url.scheme(), "http" | "https"))
                .map(|url| format!(" → {}{}", url.origin().ascii_serialization(), url.path()))
                .unwrap_or_default();
            bail!(
                "发帖入口返回 HTTP {} 重定向{}。未自动重发；请在设置中检查“网页地址（发串 / 回复）”，并先核实帖子是否已发出",
                response.status(),
                destination
            );
        }
        if !response.status().is_success() {
            bail!(
                "服务器返回 HTTP {}，发送结果未确认，请刷新核实",
                response.status()
            );
        }
        let body = response
            .text()
            .context("无法读取发送结果，请刷新核实是否已发出")?;
        parse_post_response(&body)
    }
}

fn parse_subscription_response(value: Value, expected: &str) -> Result<String> {
    match value.as_str() {
        Some(message) if message == expected => Ok(message.to_owned()),
        Some("该串不存在") => bail!("该串不存在"),
        // Never expose unrecognized server text: it may contain the private ID.
        _ => bail!("未能确认订阅操作结果，请刷新订阅列表核实"),
    }
}

fn decode_read<T: DeserializeOwned>(value: Value) -> Result<T> {
    if let Some(error) = value.as_str() {
        bail!("{}", html_to_text(error));
    }
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        let error = value
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("请求被服务器拒绝");
        bail!("{}", html_to_text(error));
    }
    serde_json::from_value(value).context("服务器返回的数据结构与预期不符")
}

fn parse_post_response(body: &str) -> Result<String> {
    static MESSAGE: OnceLock<Regex> = OnceLock::new();
    let regex = MESSAGE.get_or_init(|| {
        Regex::new(r#"(?is)<p\b[^>]*\bclass\s*=\s*[\"'](success|error)[\"'][^>]*>(.*?)</p\s*>"#)
            .unwrap()
    });
    // A server error wins if an unusual response contains both markers.
    let mut success = None;
    for capture in regex.captures_iter(body) {
        let text = html_to_text(&capture[2]);
        if capture[1].eq_ignore_ascii_case("error") {
            bail!(
                "{}",
                if text.is_empty() {
                    "服务器拒绝了发送"
                } else {
                    &text
                }
            );
        }
        success = Some(text);
    }
    success
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("未能确认发送结果，请刷新原串或版面核实后再决定是否重试"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn subscription_ids_are_validated_without_requiring_uuid_syntax() {
        assert_eq!(
            validate_subscription_id("  old-id-订阅  ").unwrap(),
            "old-id-订阅"
        );
        assert!(validate_subscription_id("").is_err());
        assert!(validate_subscription_id(" \n\t ").is_err());
        assert!(validate_subscription_id(&"岛".repeat(MAX_SUBSCRIPTION_ID_CHARS)).is_ok());
        assert!(validate_subscription_id(&"岛".repeat(MAX_SUBSCRIPTION_ID_CHARS + 1)).is_err());
        let api = ApiClient::new(None).unwrap();
        assert!(api.subscriptions("", 1).is_err());
        assert!(api.subscriptions("private-id", 0).is_err());
        assert!(api.add_subscription("private-id", 0).is_err());
        assert!(api.remove_subscription("", 123).is_err());
    }

    #[test]
    fn subscription_changes_require_the_documented_success_response() {
        for success in ["订阅大成功→_→", "取消订阅成功!"] {
            assert_eq!(
                parse_subscription_response(json!(success), success).unwrap(),
                success
            );
            for unknown in [
                json!("操作成功"),
                json!("private-subscription-id"),
                json!("订阅大成功→_→，但是失败"),
                json!({"success":true}),
                json!({"success":false,"error":"private-subscription-id"}),
                json!([]),
                json!(null),
            ] {
                let error = parse_subscription_response(unknown, success).unwrap_err();
                assert!(!format!("{error:#}").contains("private-subscription-id"));
            }
        }
        assert!(
            parse_subscription_response(json!("该串不存在"), "订阅大成功→_→")
                .unwrap_err()
                .to_string()
                .contains("该串不存在")
        );
    }

    #[test]
    fn subscriptions_use_get_and_post_forms_and_decode_string_numbers() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for body in [
                r#"[{"id":"123","fid":"4","reply_count":"7","admin":"1","content":"feed item"}]"#,
                r#""订阅大成功→_→""#,
                r#""取消订阅成功!""#,
                "[]",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                requests.push(read_mock_request(&mut stream));
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
            requests
        });
        let api = ApiClient::new(None).unwrap();
        let base = format!("http://{address}");
        let uuid = "test & 订阅";
        let posts = api
            .subscriptions_at(&format!("{base}/feed"), uuid, 2)
            .unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].id, 123);
        assert_eq!(posts[0].reply_count, 7);
        assert!(posts[0].admin);
        api.change_subscription(&format!("{base}/addFeed"), uuid, 123, "订阅大成功→_→")
            .unwrap();
        api.change_subscription(&format!("{base}/delFeed"), uuid, 123, "取消订阅成功!")
            .unwrap();
        assert!(
            api.subscriptions_at(&format!("{base}/feed"), uuid, 3)
                .unwrap()
                .is_empty()
        );
        let requests = server.join().unwrap();
        let encoded = "uuid=test+%26+%E8%AE%A2%E9%98%85";
        assert!(requests[0].starts_with(&format!("GET /feed?{encoded}&page=2 HTTP/1.1")));
        assert!(requests[1].starts_with(&format!("POST /addFeed?{encoded} HTTP/1.1")));
        assert!(requests[2].starts_with(&format!("POST /delFeed?{encoded} HTTP/1.1")));
        for request in &requests[1..3] {
            assert!(request.ends_with("tid=123"));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("content-type: application/x-www-form-urlencoded")
            );
        }
        assert!(
            requests
                .iter()
                .all(|request| !request.to_ascii_lowercase().contains("cookie:"))
        );
    }

    #[test]
    fn subscription_transport_errors_do_not_expose_private_ids() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for (status, body) in [
                (
                    "200 OK",
                    r#"{"success":false,"error":"secret-subscription-id"}"#,
                ),
                ("200 OK", "invalid json secret-subscription-id"),
                ("500 Internal Server Error", "secret-subscription-id"),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                read_mock_request(&mut stream);
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
        });
        let api = ApiClient::new(None).unwrap();
        let endpoint = format!("http://{address}/feed");
        for _ in 0..3 {
            let error = api
                .subscriptions_at(&endpoint, "secret-subscription-id", 1)
                .unwrap_err();
            assert!(!format!("{error:#}").contains("secret-subscription-id"));
        }
        server.join().unwrap();
        // The closed listener forces a reqwest error that normally includes its URL.
        let error = api
            .subscriptions_at(&endpoint, "secret-subscription-id", 1)
            .unwrap_err();
        assert!(!format!("{error:#}").contains("secret-subscription-id"));
    }

    #[test]
    fn references_decode_posts_without_inventing_an_enclosing_thread() {
        for id in [json!(50039112), json!("50039112")] {
            let preview: ReferencePreview = decode_read(json!({
                "id": id,
                "img": "",
                "ext": "",
                "now": "2022-06-21(二)08:06:58",
                "user_hash": "JzeObtU",
                "name": "无名氏",
                "title": "无标题",
                "content": "( ´∀`)回家啦",
                "sage": 0,
                "status": "n",
                "admin": 0,
                "fid": 4
            }))
            .unwrap();
            assert_eq!(preview.post.id, 50039112);
            assert_eq!(preview.post.content, "( ´∀`)回家啦");
            assert_eq!(preview.post.fid, 4);
            assert_eq!(preview.thread_id, None);
        }
    }

    #[test]
    fn references_preserve_server_errors_and_validate_ids_before_requesting() {
        let client = ApiClient::new(None).unwrap();
        assert!(
            client
                .reference(0)
                .unwrap_err()
                .to_string()
                .contains("大于零")
        );
        for error in [
            json!("该串不存在"),
            json!({"success":false,"error":"该串不存在"}),
        ] {
            assert!(
                decode_read::<ReferencePreview>(error)
                    .unwrap_err()
                    .to_string()
                    .contains("该串不存在")
            );
        }
        assert!(decode_read::<ReferencePreview>(json!({"content":"missing id"})).is_err());
    }

    #[test]
    fn website_origin_defaults_and_normalizes_without_insecure_urls() {
        assert_eq!(
            normalize_website_base("").unwrap(),
            "https://www.nmbxd1.com/"
        );
        assert_eq!(
            normalize_website_base(" www.nmbxd2.com ").unwrap(),
            "https://www.nmbxd2.com/"
        );
        let client = ApiClient::with_website_base(None, "https://example.com:8443/").unwrap();
        assert_eq!(client.website_base(), "https://example.com:8443/");
        for invalid in [
            "http://example.com",
            "https://user:secret@example.com",
            "https://example.com/api/",
            "https://example.com/?token=secret",
            "https://example.com/#fragment",
            "https://",
        ] {
            assert!(normalize_website_base(invalid).is_err());
        }
    }

    // A local socket fixture exercises the actual HTTP transport, never the island.
    fn read_mock_request_bytes(stream: &mut std::net::TcpStream) -> Vec<u8> {
        use std::io::Read;
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            assert!(count > 0, "request ended before its body");
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        bytes
    }

    fn read_mock_request(stream: &mut std::net::TcpStream) -> String {
        String::from_utf8(read_mock_request_bytes(stream)).unwrap()
    }

    fn test_image(format: image::ImageFormat) -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(2, 2)
            .write_to(&mut bytes, format)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn attachments_validate_actual_images_and_normalize_filename() {
        for (format, filename, mime) in [
            (image::ImageFormat::Jpeg, "photo.jpg", "image/jpeg"),
            (image::ImageFormat::Png, "photo.png", "image/png"),
            (image::ImageFormat::Gif, "photo.gif", "image/gif"),
        ] {
            let bytes = test_image(format);
            let image = ImageAttachment::new("C:\\private\\photo\r\n.fake", bytes.clone()).unwrap();
            assert_eq!(image.filename(), filename);
            assert_eq!(image.mime_type(), mime);
            assert_eq!(image.bytes(), bytes);
            assert_eq!(image.size(), bytes.len());
        }
        assert!(ImageAttachment::new("empty.png", vec![]).is_err());
        assert!(ImageAttachment::new("fake.png", b"not an image".to_vec()).is_err());
        assert!(ImageAttachment::new("broken.gif", b"GIF89a".to_vec()).is_err());
        assert!(ImageAttachment::new("large.png", vec![0; MAX_IMAGE_BYTES + 1]).is_err());
        assert!(
            ImageAttachment::new("unsupported.webp", test_image(image::ImageFormat::WebP)).is_err()
        );
        let image = ImageAttachment::new("../\n.png", test_image(image::ImageFormat::Png)).unwrap();
        assert_eq!(image.filename(), "attachment.png");
    }

    #[test]
    fn post_and_reply_use_the_configured_origin_and_encoded_forms() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                requests.push(read_mock_request(&mut stream));
                let body = "<p class='success'>ok</p>";
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
            requests
        });
        let mut api = ApiClient::new(Some("test-only-cookie")).unwrap();
        // HTTP is allowed only inside this private local fixture, not user settings.
        api.website_base = format!("http://{address}/");
        assert_eq!(
            api.post_thread(117, "", "API test", "hello & 中文")
                .unwrap(),
            "ok"
        );
        assert_eq!(api.reply(123, "", "", ">>No.456\nreply").unwrap(), "ok");
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("POST /home/forum/doPostThread.html HTTP/1.1"));
        assert!(
            requests[0]
                .ends_with("fid=117&name=&title=API+test&content=hello+%26+%E4%B8%AD%E6%96%87")
        );
        assert!(requests[1].starts_with("POST /home/forum/doReplyThread.html HTTP/1.1"));
        assert!(requests[1].ends_with("resto=123&name=&title=&content=%3E%3ENo.456%0Areply"));
    }

    #[test]
    fn image_posts_use_multipart_and_preserve_server_errors() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for body in [
                "<p class='success'>发串成功</p>",
                "<p class='error'>图片被拒绝</p>",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                requests.push(read_mock_request_bytes(&mut stream));
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .unwrap();
            }
            requests
        });
        let bytes = test_image(image::ImageFormat::Png);
        let image = ImageAttachment::new("/private/photo.png", bytes.clone()).unwrap();
        let mut api = ApiClient::new(Some("test-only-cookie")).unwrap();
        api.website_base = format!("http://{address}/");
        assert_eq!(
            api.post_thread_with_image(117, "", "", "", Some(&image))
                .unwrap(),
            "发串成功"
        );
        assert!(
            api.reply_with_image(123, "", "", ">>No.456\nreply", Some(&image))
                .unwrap_err()
                .to_string()
                .contains("图片被拒绝")
        );
        let requests = server.join().unwrap();
        for request in &requests {
            let text = String::from_utf8_lossy(request);
            assert!(text.contains("multipart/form-data; boundary="));
            assert!(text.contains("name=\"image\"; filename=\"photo.png\""));
            assert!(
                text.contains("Content-Type: image/png")
                    || text.contains("content-type: image/png")
            );
            assert!(request.windows(bytes.len()).any(|part| part == bytes));
            assert!(!text.contains("/private/"));
        }
        let post = String::from_utf8_lossy(&requests[0]);
        assert!(post.starts_with("POST /home/forum/doPostThread.html HTTP/1.1"));
        assert!(post.contains("name=\"fid\"\r\n\r\n117\r\n"));
        assert!(post.contains("name=\"content\"\r\n\r\n\r\n"));
        assert!(!post.contains("name=\"resto\""));
        let reply = String::from_utf8_lossy(&requests[1]);
        assert!(reply.starts_with("POST /home/forum/doReplyThread.html HTTP/1.1"));
        assert!(reply.contains("name=\"resto\"\r\n\r\n123\r\n"));
        assert!(reply.contains(">>No.456\nreply"));
        assert!(!reply.contains("name=\"fid\""));
    }

    #[test]
    fn redirect_does_not_replay_post_or_disclose_location_tokens() {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let destination = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let target = destination.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request = read_mock_request_bytes(&mut stream);
            write!(stream, "HTTP/1.1 307 Temporary Redirect\r\nLocation: http://{target}/moved?token=do-not-display\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            request
        });
        let mut api = ApiClient::new(Some("test-only-cookie")).unwrap();
        api.website_base = format!("http://{address}/");
        let image = ImageAttachment::new("image.gif", test_image(image::ImageFormat::Gif)).unwrap();
        let error = api
            .post_thread_with_image(117, "", "", "test", Some(&image))
            .unwrap_err()
            .to_string();
        assert!(error.contains("307"));
        assert!(error.contains("网页地址"));
        assert!(!error.contains("do-not-display"));
        assert!(!error.contains("test-only-cookie"));
        assert!(server.join().unwrap().starts_with(b"POST "));
        destination.set_nonblocking(true).unwrap();
        assert_eq!(
            destination.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn reads_gzip_responses() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request).unwrap();
            let body: &[u8] = &[
                31, 139, 8, 0, 0, 0, 0, 0, 0, 255, 139, 142, 5, 0, 41, 187, 76, 13, 2, 0, 0, 0,
            ];
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Encoding: gzip\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            stream.write_all(body).unwrap();
        });
        let api = ApiClient::new(None).unwrap();
        let groups: Vec<ForumGroup> = api
            .read(api.client.get(format!("http://{address}/")))
            .unwrap();
        assert!(groups.is_empty());
        server.join().unwrap();
    }

    #[test]
    fn api_errors_are_not_misread_as_empty_posts() {
        assert!(
            decode_read::<Post>(json!("该串不存在"))
                .unwrap_err()
                .to_string()
                .contains("该串不存在")
        );
        assert!(
            decode_read::<Vec<Post>>(
                json!({"success":false,"error":"必须登入领取饼干后才可以访问"})
            )
            .unwrap_err()
            .to_string()
            .contains("饼干")
        );
        assert!(decode_read::<Post>(json!({"unrelated":1})).is_err());
    }

    #[test]
    fn posting_requires_an_explicit_success_marker() {
        assert_eq!(
            parse_post_response("<p class=\"success\">发串成功</p>").unwrap(),
            "发串成功"
        );
        assert!(
            parse_post_response("<p class='error'>未应用饼干</p>")
                .unwrap_err()
                .to_string()
                .contains("未应用饼干")
        );
        assert!(parse_post_response("<html>HTTP 200 but unknown response</html>").is_err());
        assert!(
            parse_post_response("<p class='success'>ok</p><p class='error'>failure</p>").is_err()
        );
    }

    #[test]
    fn credentials_are_not_in_debug_and_validation_does_not_send() {
        let client = ApiClient::new(Some("userhash=private-token; unrelated=discarded")).unwrap();
        assert!(client.has_cookie());
        assert!(!format!("{client:?}").contains("private-token"));
        assert!(client.reply(123, "", "", "  ").is_err());
        assert!(
            ApiClient::new(None)
                .unwrap()
                .reply(123, "", "", "content")
                .is_err()
        );
        assert!(ApiClient::new(Some("bad\nvalue")).is_err());
    }
}
