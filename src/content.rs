//! Safe, plain-text rendering with clickable ranges for the API's HTML fragments.
use regex::Regex;
use std::{ops::Range, sync::OnceLock};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkTarget {
    Reference(u64),
    External(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentLink {
    /// UTF-8 byte offsets into `ParsedContent::text`.
    pub range: Range<usize>,
    pub target: LinkTarget,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ParsedContent {
    pub text: String,
    /// Sorted, nonoverlapping ranges; repeated references remain separate links.
    pub links: Vec<ContentLink>,
}

/// Render only text and simple block breaks. Markup is never executed. Anchor
/// destinations survive rendering, while plain references and web URLs are
/// discovered in text outside those anchors.
pub fn parse_content(html: &str) -> ParsedContent {
    static PLAIN_LINK: OnceLock<Regex> = OnceLock::new();
    let mut parsed = render_html(html);
    let text = &parsed.text;
    let links = &mut parsed.links;
    // Match visible strings only, after decoding HTML entities. Anchor ranges
    // retain priority even if their label resembles a different destination.
    let plain = PLAIN_LINK.get_or_init(|| {
        Regex::new(r#"(?i)https?://[^\s<>\"']+|(?:>>\s*(?:No\.\s*)?|No\.\s*)([0-9]+)"#).unwrap()
    });
    let anchor_count = links.len();
    for capture in plain.captures_iter(text) {
        let matched = capture.get(0).unwrap();
        if matched
            .as_str()
            .get(..3)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("No."))
            && text[..matched.start()]
                .chars()
                .next_back()
                .is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            continue;
        }
        let (end, target) = if let Some(id) = capture.get(1) {
            let Some(id) = positive_id(id.as_str()) else {
                continue;
            };
            (matched.end(), LinkTarget::Reference(id))
        } else {
            let url = trim_url_punctuation(matched.as_str());
            let Some(target) = link_target(url) else {
                continue;
            };
            (matched.start() + url.len(), target)
        };
        let range = matched.start()..end;
        let next_anchor =
            links[..anchor_count].partition_point(|link| link.range.end <= range.start);
        if links[..anchor_count]
            .get(next_anchor)
            .is_some_and(|link| ranges_overlap(&range, &link.range))
        {
            continue;
        }
        links.push(ContentLink { range, target });
    }

    links.sort_by_key(|link| link.range.start);
    parsed
}

/// Text-only callers share the renderer without scanning for clickable links.
pub(crate) fn render_text(html: &str) -> String {
    render_html(html).text
}

fn render_html(html: &str) -> ParsedContent {
    static TAG: OnceLock<Regex> = OnceLock::new();
    let tags = TAG.get_or_init(|| {
        Regex::new(r#"(?is)<!--.*?-->|</?([a-z][a-z0-9]*)\b(?:[^'\"<>]|\"[^\"]*\"|'[^']*')*>"#)
            .unwrap()
    });
    let mut text = String::new();
    let mut links = Vec::new();
    let mut anchor: Option<(usize, LinkTarget)> = None;
    let mut hidden: Option<String> = None;
    let mut cursor = 0;
    for capture in tags.captures_iter(html) {
        let tag = capture.get(0).unwrap();
        if hidden.is_none() {
            append_text(&mut text, &html[cursor..tag.start()]);
        }
        cursor = tag.end();
        let Some(name) = capture.get(1) else { continue };
        let name = name.as_str().to_ascii_lowercase();
        let closing = tag.as_str().starts_with("</");
        if let Some(hidden_name) = &hidden {
            if closing && name == *hidden_name {
                hidden = None;
            }
            continue;
        }
        if !closing && matches!(name.as_str(), "script" | "style") {
            hidden = Some(name);
            continue;
        }
        if name == "a" {
            finish_anchor(&mut anchor, &mut links, text.len());
            if !closing {
                anchor = anchor_target(tag.as_str()).map(|target| (text.len(), target));
            }
        }
        if name == "br" && !closing
            || closing
                && matches!(
                    name.as_str(),
                    "p" | "div"
                        | "li"
                        | "h1"
                        | "h2"
                        | "h3"
                        | "h4"
                        | "h5"
                        | "h6"
                        | "blockquote"
                        | "tr"
                )
        {
            text.push('\n');
            if name == "br" {
                // The API uses nl2br-style fragments: the tag and the one
                // immediately following source newline represent one break.
                // Consume only that newline so repeated tags, extra blank
                // lines, indentation, and entity-encoded newlines survive.
                let source = &html[cursor..];
                cursor += if source.starts_with("\r\n") {
                    2
                } else if source.starts_with(['\n', '\r']) {
                    1
                } else {
                    0
                };
            }
        }
    }
    if hidden.is_none() {
        append_text(&mut text, &html[cursor..]);
    }
    finish_anchor(&mut anchor, &mut links, text.len());

    // Preserve the existing renderer's outer whitespace trimming, adjusting all
    // ranges together so offsets stay valid for CJK and emoji.
    let start = text.len() - text.trim_start().len();
    let end = text.trim_end().len();
    links.retain_mut(|link| {
        let left = link.range.start.max(start);
        let right = link.range.end.min(end);
        if left >= right {
            return false;
        }
        link.range = (left - start)..(right - start);
        true
    });
    ParsedContent {
        text: text.trim().to_owned(),
        links,
    }
}

fn append_text(output: &mut String, raw: &str) {
    output.push_str(
        &html_escape::decode_html_entities(raw)
            .replace('\u{a0}', " ")
            .replace("\r\n", "\n"),
    );
}

fn finish_anchor(
    anchor: &mut Option<(usize, LinkTarget)>,
    links: &mut Vec<ContentLink>,
    end: usize,
) {
    if let Some((start, target)) = anchor.take()
        && start < end
    {
        links.push(ContentLink {
            range: start..end,
            target,
        });
    }
}

fn anchor_target(tag: &str) -> Option<LinkTarget> {
    static ATTRIBUTE: OnceLock<Regex> = OnceLock::new();
    let attributes = ATTRIBUTE.get_or_init(|| {
        Regex::new(
            r#"(?is)([a-z_:][a-z0-9_.:-]*)\s*=\s*(?:\"([^\"]*)\"|'([^']*)'|([^\s'\"=<>`]+))"#,
        )
        .unwrap()
    });
    for capture in attributes.captures_iter(tag) {
        if capture[1].eq_ignore_ascii_case("href") {
            let value = capture
                .get(2)
                .or_else(|| capture.get(3))
                .or_else(|| capture.get(4))?;
            return link_target(&html_escape::decode_html_entities(value.as_str()));
        }
    }
    None
}

fn link_target(value: &str) -> Option<LinkTarget> {
    let value = value.trim();
    if value.chars().any(char::is_control) {
        return None;
    }
    // Only these relative paths are meaningful inside an API fragment.
    if value.starts_with("/t/") && !value.contains('\\') {
        let url = reqwest::Url::parse("https://www.nmbxd1.com/")
            .ok()?
            .join(value)
            .ok()?;
        return reference_path(&url).map(LinkTarget::Reference);
    }
    if !(value
        .get(..7)
        .is_some_and(|s| s.eq_ignore_ascii_case("http://"))
        || value
            .get(..8)
            .is_some_and(|s| s.eq_ignore_ascii_case("https://")))
    {
        return None;
    }
    let url = reqwest::Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    if url.host_str().is_some_and(is_island_host)
        && let Some(id) = reference_path(&url)
    {
        return Some(LinkTarget::Reference(id));
    }
    Some(LinkTarget::External(url.to_string()))
}

fn is_island_host(host: &str) -> bool {
    let host = host.strip_prefix("www.").unwrap_or(host);
    matches!(
        host,
        "nmbxd.com"
            | "nmbxd1.com"
            | "nmbxd2.com"
            | "nmbxd3.com"
            | "nmb.best"
            | "adnmb.com"
            | "adnmb1.com"
            | "adnmb2.com"
    )
}

fn reference_path(url: &reqwest::Url) -> Option<u64> {
    let thread_id = positive_id(url.path().strip_prefix("/t/")?.trim_end_matches('/'))?;
    // A reply fragment identifies the reply more precisely than the thread URL.
    if let Some(fragment) = url.fragment() {
        for prefix in ["r", "reply", "reply_", "post_", "No."] {
            if let Some(id) = fragment.strip_prefix(prefix).and_then(positive_id) {
                return Some(id);
            }
        }
        if let Some(id) = positive_id(fragment) {
            return Some(id);
        }
    }
    Some(thread_id)
}

fn positive_id(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok().filter(|id| *id != 0)
}

fn trim_url_punctuation(mut value: &str) -> &str {
    loop {
        let Some(last) = value.chars().last() else {
            return value;
        };
        let punctuation = matches!(
            last,
            '.' | ','
                | '!'
                | '?'
                | ';'
                | ':'
                | '。'
                | '，'
                | '！'
                | '？'
                | '；'
                | '：'
                | '、'
                | '»'
                | '”'
                | '’'
        );
        let unbalanced = [
            ('(', ')'),
            ('[', ']'),
            ('{', '}'),
            ('（', '）'),
            ('【', '】'),
        ]
        .iter()
        .any(|&(open, close)| {
            last == close && value.matches(close).count() > value.matches(open).count()
        });
        if !punctuation && !unbalanced {
            return value;
        }
        value = &value[..value.len() - last.len_utf8()];
    }
}

fn ranges_overlap(a: &Range<usize>, b: &Range<usize>) -> bool {
    a.start < b.end && b.start < a.end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(parsed: &ParsedContent) -> Vec<&str> {
        parsed
            .links
            .iter()
            .map(|link| &parsed.text[link.range.clone()])
            .collect()
    }

    #[test]
    fn preserves_layout_entities_and_utf8_offsets() {
        let parsed = parse_content(
            " <p>中文 😀 &amp; 👩‍💻<br/>  &gt;&gt;No.12</p>&lt;b&gt;literal&lt;/b&gt; ",
        );
        assert_eq!(parsed.text, "中文 😀 & 👩‍💻\n  >>No.12\n<b>literal</b>");
        assert_eq!(labels(&parsed), vec![">>No.12"]);
        assert_eq!(parsed.links[0].target, LinkTarget::Reference(12));
    }

    #[test]
    fn paired_source_breaks_preserve_layout_and_utf8_link_ranges() {
        let parsed = parse_content(
            " 中文😀<br />\r\n<a href='/t/42'>引用🎉</a><br>\n&gt;&gt;No.12<br>\nhttps://example.com/路径 ",
        );
        assert_eq!(
            parsed.text,
            "中文😀\n引用🎉\n>>No.12\nhttps://example.com/路径"
        );
        assert_eq!(
            labels(&parsed),
            vec!["引用🎉", ">>No.12", "https://example.com/路径"]
        );
        assert_eq!(parsed.links[0].target, LinkTarget::Reference(42));
        assert_eq!(parsed.links[1].target, LinkTarget::Reference(12));
        assert_eq!(parsed.links[0].range.start, "中文😀\n".len());
        assert!(matches!(parsed.links[2].target, LinkTarget::External(_)));
    }

    #[test]
    fn recognizes_duplicate_references_but_not_bare_numbers() {
        let parsed = parse_content(
            "123 >>123 >>No.123 No.123 >>No.0 >>18446744073709551616 xNo.4 中文No.234",
        );
        assert_eq!(
            labels(&parsed),
            vec![">>123", ">>No.123", "No.123", "No.234"]
        );
        assert!(
            parsed.links[..3]
                .iter()
                .all(|link| link.target == LinkTarget::Reference(123))
        );
        assert_eq!(parsed.links[3].target, LinkTarget::Reference(234));
    }

    #[test]
    fn preserves_anchor_target_and_label_without_overlapping_links() {
        let parsed = parse_content(
            "<a title=\"a > b\" href='/t/42'>&gt;&gt;No.12 😀</a> <a href='https://example.com/?x=1&amp;y=2'>网页</a>",
        );
        assert_eq!(labels(&parsed), vec![">>No.12 😀", "网页"]);
        assert_eq!(parsed.links[0].target, LinkTarget::Reference(42));
        assert_eq!(
            parsed.links[1].target,
            LinkTarget::External("https://example.com/?x=1&y=2".into())
        );
    }

    #[test]
    fn rejects_unsafe_destinations_and_hides_active_content() {
        let parsed = parse_content(
            "<a href='javascript:alert(1)'>one</a> <a href='data:text/plain,x'>two</a> <a href='file:///etc/passwd'>three</a> <a href='https://user:secret@example.com'>four</a><script>https://evil.test >>4</script><style>No.5</style><script>No.6",
        );
        assert_eq!(parsed.text, "one two three four");
        assert!(parsed.links.is_empty());
        assert!(
            parse_content("<a title=\"href='https://evil.test'\">label</a>")
                .links
                .is_empty()
        );
    }

    #[test]
    fn trims_sentence_punctuation_and_keeps_balanced_url_parentheses() {
        let parsed = parse_content(
            "😀 https://example.com/a(b). (https://example.com/x)。 https://example.com/路径，",
        );
        assert_eq!(
            labels(&parsed),
            vec![
                "https://example.com/a(b)",
                "https://example.com/x",
                "https://example.com/路径"
            ]
        );
    }

    #[test]
    fn island_paths_and_reply_fragments_stay_internal() {
        let parsed = parse_content(
            "<a href='https://www.nmbxd1.com/t/123#r456'>引用</a> https://nmbxd2.com/t/77 <a href='https://nmbxd1.com.evil.test/t/9'>外部</a>",
        );
        assert_eq!(parsed.links[0].target, LinkTarget::Reference(456));
        assert_eq!(parsed.links[1].target, LinkTarget::Reference(77));
        assert!(matches!(parsed.links[2].target, LinkTarget::External(_)));
    }

    #[test]
    fn trimming_whitespace_and_nested_markup_keeps_ranges_valid() {
        let parsed = parse_content(" \n<a href='/t/8'> <b>😀</b> </a> \n");
        assert_eq!(parsed.text, "😀");
        assert_eq!(parsed.links[0].range, 0.."😀".len());
        let parsed = parse_content("<a href='/t/8'> </a>");
        assert!(parsed.text.is_empty());
        assert!(parsed.links.is_empty());
    }
}
