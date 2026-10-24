//! Wire models and plain text rendering for X 岛's mixed string/number API.
use regex::Regex;
use serde::{Deserialize, Deserializer, Serialize};
use std::sync::OnceLock;

fn integer<'de, D: Deserializer<'de>>(d: D) -> Result<i64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Integer(i64),
        Text(String),
    }
    match Number::deserialize(d)? {
        Number::Integer(n) => Ok(n),
        Number::Text(s) => s.parse().map_err(serde::de::Error::custom),
    }
}

fn unsigned<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Number {
        Integer(u64),
        Text(String),
    }
    match Number::deserialize(d)? {
        Number::Integer(n) => Ok(n),
        Number::Text(s) => s.parse().map_err(serde::de::Error::custom),
    }
}

fn flag<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Flag {
        Bool(bool),
        Integer(i64),
        Text(String),
        Null,
    }
    match Flag::deserialize(d)? {
        Flag::Bool(v) => Ok(v),
        Flag::Integer(v) => Ok(v != 0),
        Flag::Text(v) => match v.as_str() {
            "true" => Ok(true),
            "false" | "" => Ok(false),
            _ => v
                .parse::<i64>()
                .map(|n| n != 0)
                .map_err(serde::de::Error::custom),
        },
        Flag::Null => Ok(false),
    }
}

fn nullable_vec<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Vec<T>, D::Error> {
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct ForumGroup {
    #[serde(deserialize_with = "integer")]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "nullable_vec")]
    pub forums: Vec<Forum>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Forum {
    #[serde(deserialize_with = "integer")]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "showName")]
    pub show_name: String,
    #[serde(default)]
    pub msg: String,
    #[serde(default, deserialize_with = "unsigned")]
    pub interval: u64,
}

impl Forum {
    pub fn display_name(&self) -> String {
        html_to_text(if self.show_name.trim().is_empty() {
            &self.name
        } else {
            &self.show_name
        })
    }
}

/// Timelines aggregate multiple forums and have their own IDs and page limits.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct Timeline {
    #[serde(deserialize_with = "integer")]
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub notice: String,
    #[serde(default, deserialize_with = "page_count")]
    pub max_page: u32,
}

fn page_count<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    unsigned(d)?.try_into().map_err(serde::de::Error::custom)
}

impl Timeline {
    pub fn display_name(&self) -> String {
        html_to_text(if self.display_name.trim().is_empty() {
            &self.name
        } else {
            &self.display_name
        })
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
pub struct Post {
    #[serde(deserialize_with = "unsigned")]
    pub id: u64,
    #[serde(default, deserialize_with = "integer")]
    pub fid: i64,
    #[serde(
        default,
        rename = "ReplyCount",
        alias = "reply_count",
        deserialize_with = "unsigned"
    )]
    pub reply_count: u64,
    #[serde(default)]
    pub img: String,
    #[serde(default)]
    pub ext: String,
    #[serde(default)]
    pub now: String,
    #[serde(default)]
    pub user_hash: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default, deserialize_with = "flag")]
    pub sage: bool,
    #[serde(default, deserialize_with = "flag")]
    pub admin: bool,
    #[serde(
        default,
        rename = "Replies",
        alias = "replies",
        deserialize_with = "nullable_vec"
    )]
    pub replies: Vec<Post>,
}

impl Post {
    pub fn text(&self) -> String {
        html_to_text(&self.content)
    }

    /// This service notice is repeated on thread pages and is not a real reply.
    pub fn is_tip(&self) -> bool {
        self.id == 9_999_999 && self.user_hash == "Tips"
    }

    pub fn image_url(&self, thumbnail: bool) -> Option<String> {
        if self.img.is_empty()
            || !self.img.split('/').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
            || ![".jpg", ".jpeg", ".png", ".gif", ".webp"]
                .contains(&self.ext.to_ascii_lowercase().as_str())
        {
            return None;
        }
        // Never interpret arbitrary API text as an executable URI scheme.
        Some(format!(
            "https://image.nmb.best/{}/{}{}",
            if thumbnail { "thumb" } else { "image" },
            self.img,
            self.ext
        ))
    }
}

/// Render the API's small HTML fragments as text. Decode entities *after*
/// removing markup, so escaped angle brackets in users' text remain visible.
pub fn html_to_text(html: &str) -> String {
    crate::content::render_text(html)
}

/// References in visible content, deduplicated in their original order.
pub fn references(content: &str) -> Vec<u64> {
    static REFERENCE: OnceLock<Regex> = OnceLock::new();
    let text = html_to_text(content);
    let mut ids = Vec::new();
    for c in REFERENCE
        .get_or_init(|| Regex::new(r"(?i)>>\s*No\.\s*([0-9]+)").unwrap())
        .captures_iter(&text)
    {
        if let Ok(id) = c[1].parse::<u64>()
            && id != 0
            && !ids.contains(&id)
        {
            ids.push(id);
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_paths_are_safe_and_support_date_directories() {
        let mut post = Post {
            img: "2022-06-18/62acedc59ef24".into(),
            ext: ".png".into(),
            ..Post::default()
        };
        assert_eq!(
            post.image_url(true).as_deref(),
            Some("https://image.nmb.best/thumb/2022-06-18/62acedc59ef24.png")
        );
        assert_eq!(
            post.image_url(false).as_deref(),
            Some("https://image.nmb.best/image/2022-06-18/62acedc59ef24.png")
        );
        for path in [
            "../secret",
            "/absolute",
            "foo?bar",
            "foo#bar",
            "foo//bar",
            "https://other.test/image",
        ] {
            post.img = path.into();
            assert!(post.image_url(true).is_none());
        }
        assert_eq!(html_to_text("😀 &#x1F389; 👩‍💻 ❤️"), "😀 🎉 👩‍💻 ❤️");
    }

    #[test]
    fn mixed_wire_types_and_partial_replies() {
        let post: Post = serde_json::from_str(r#"{"id":"123","fid":4,"ReplyCount":"2","admin":"1","sage":0,"Replies":[{"id":124,"content":"hi"}]}"#).unwrap();
        assert_eq!(post.id, 123);
        assert_eq!(post.reply_count, 2);
        assert!(post.admin);
        assert!(!post.sage);
        assert_eq!(post.replies[0].fid, 0);
        assert!(serde_json::from_str::<Post>(r#"{"success":false,"error":"missing"}"#).is_err());
        assert!(serde_json::from_str::<Post>(r#"{"id":-1}"#).is_err());
    }

    #[test]
    fn forums_accept_negative_sentinel() {
        let forum: Forum =
            serde_json::from_str(r#"{"id":"-1","name":"时间线","showName":"<b>时间&amp;线</b>"}"#)
                .unwrap();
        assert_eq!(forum.id, -1);
        assert_eq!(forum.display_name(), "时间&线");
    }

    #[test]
    fn timelines_decode_metadata_and_fallback_display_names() {
        for (id, limit) in [("1", "20"), ("\"1\"", "\"20\"")] {
            let timeline: Timeline = serde_json::from_str(&format!(
                r#"{{"id":{id},"name":"综合线","display_name":"<b>综合&amp;线</b>","notice":"主时间线","max_page":{limit}}}"#,
            ))
            .unwrap();
            assert_eq!(timeline.id, 1);
            assert_eq!(timeline.max_page, 20);
            assert_eq!(timeline.notice, "主时间线");
            assert_eq!(timeline.display_name(), "综合&线");
        }
        let timeline: Timeline =
            serde_json::from_str(r#"{"id":2,"name":"创作线","display_name":" "}"#).unwrap();
        assert_eq!(timeline.display_name(), "创作线");
        assert_eq!(timeline.max_page, 0);
        assert!(serde_json::from_str::<Timeline>(r#"{"id":1,"max_page":-1}"#).is_err());
        assert!(serde_json::from_str::<Timeline>(r#"{"id":1,"max_page":4294967296}"#).is_err());
    }

    #[test]
    fn renders_entities_and_line_breaks_without_eating_literal_markup() {
        assert_eq!(
            html_to_text(
                "<p>A &amp; B<br/>第二行</p><script>hidden</script><style>hidden</style>&lt;b&gt;literal&lt;/b&gt;"
            ),
            "A & B\n第二行\n<b>literal</b>"
        );
        assert_eq!(
            html_to_text("<a title=\"a > b\" href=\"/t/1\">&gt;&gt;No.12</a>"),
            ">>No.12"
        );
    }

    #[test]
    fn paired_html_breaks_render_once_in_both_paths() {
        for (html, expected) in [
            ("甲<br>\n乙", "甲\n乙"),
            ("甲<br />\r\n乙", "甲\n乙"),
            ("甲<BR/>\r乙", "甲\n乙"),
            ("甲<br>乙", "甲\n乙"),
            ("甲<br>\n<br />\r\n乙", "甲\n\n乙"),
            ("甲<br><br>乙", "甲\n\n乙"),
            ("甲<br>\n\n乙", "甲\n\n乙"),
            ("甲<br>\n  乙", "甲\n  乙"),
            ("甲\n\n乙", "甲\n\n乙"),
            ("甲\r\n\r\n乙", "甲\n\n乙"),
            ("甲<br>&#10;乙", "甲\n\n乙"),
            ("甲&lt;br&gt;\n乙", "甲<br>\n乙"),
        ] {
            assert_eq!(html_to_text(html), expected, "text-only: {html:?}");
            assert_eq!(
                crate::content::parse_content(html).text,
                expected,
                "clickable: {html:?}"
            );
        }
    }

    #[test]
    fn references_are_visible_ordered_unique_and_checked() {
        assert_eq!(
            references(
                "<a href='/t/5'>&gt;&gt;No.12</a> >>No.2 >>No.12 >>No.0 >>No.18446744073709551616<script>&gt;&gt;No.3</script>"
            ),
            vec![12, 2]
        );
    }
}
