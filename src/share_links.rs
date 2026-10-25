//! Share links and explicit navigation targets.
use reqwest::Url;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OpenTarget {
    Thread(u64),
    Message(u64),
}

/// A bare number is a message ID: it does not establish an enclosing thread.
pub fn parse_target(input: &str, website_base: &str) -> Result<OpenTarget, String> {
    let input = input.trim();
    let invalid = || "请输入有效的串链接、串号或消息 ID".to_owned();
    if input.is_empty() || input.chars().any(char::is_control) || input.contains('\\') {
        return Err(invalid());
    }

    let number = input.strip_prefix(">>").unwrap_or(input).trim_start();
    let number = if number
        .get(..3)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("No."))
    {
        number[3..].trim_start()
    } else {
        number
    };
    if let Some(id) = positive_id(number) {
        return Ok(OpenTarget::Message(id));
    }

    let url = Url::parse(input).map_err(|_| invalid())?;
    // App message links preserve a reply ID without inventing its parent.
    if url.scheme() == "rdnmb"
        && url.host_str() == Some("message")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
    {
        return url
            .path()
            .strip_prefix('/')
            .and_then(positive_id)
            .map(OpenTarget::Message)
            .ok_or_else(invalid);
    }
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(invalid());
    }
    let host = url.host_str().ok_or_else(invalid)?;
    let configured = Url::parse(website_base).ok();
    if !is_island_host(host)
        && !configured
            .as_ref()
            .and_then(Url::host_str)
            .is_some_and(|configured| configured == host)
    {
        return Err("链接不属于 X 岛或设置中的网页站点".to_owned());
    }

    let path = url.path().strip_suffix('/').unwrap_or(url.path());
    // The website's “只看 PO” link uses /Forum/po/id/{thread}.html.
    let id = path
        .strip_prefix("/t/")
        .or_else(|| path.strip_prefix("/Forum/po/id/"))
        .and_then(|id| positive_id(id.strip_suffix(".html").unwrap_or(id)))
        .ok_or_else(invalid)?;

    // Match fragments already recognized by content.rs, and the website's
    // actual reply links: /t/{thread}?page={page}&r={message}.
    if let Some(fragment) = url.fragment() {
        for prefix in ["r", "reply", "reply_", "post_", "No.", ""] {
            if let Some(id) = fragment.strip_prefix(prefix).and_then(positive_id) {
                return Ok(OpenTarget::Message(id));
            }
        }
    }
    for (key, value) in url.query_pairs() {
        if key == "r"
            && let Some(id) = positive_id(&value)
        {
            return Ok(OpenTarget::Message(id));
        }
    }
    Ok(OpenTarget::Thread(id))
}

/// Recover the verified thread context of a shared website reply link.
pub fn enclosing_thread(input: &str, website_base: &str) -> Option<u64> {
    parse_target(input, website_base).ok()?;
    let mut url = Url::parse(input.trim()).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    url.set_fragment(None);
    url.set_query(None);
    match parse_target(url.as_str(), website_base).ok()? {
        OpenTarget::Thread(id) => Some(id),
        OpenTarget::Message(_) => None,
    }
}

pub fn thread_url(website_base: &str, id: u64) -> String {
    format!("{}/t/{id}", website_base.trim_end_matches('/'))
}

/// App-specific link for a message whose enclosing thread is unknown.
/// Paste this into the app's open dialog; browser links need the parent ID.
pub fn message_url(_website_base: &str, id: u64) -> String {
    format!("rdnmb://message/{id}")
}

fn positive_id(value: &str) -> Option<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse::<u64>().ok().filter(|id| *id > 0)
}

fn is_island_host(host: &str) -> bool {
    matches!(
        host.strip_prefix("www.").unwrap_or(host),
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

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://www.nmbxd1.com/";

    #[test]
    fn numbers_and_references_are_messages() {
        for input in ["123", " 123\n", "No.123", "no. 123", ">>No.123", ">> 123"] {
            assert_eq!(parse_target(input, BASE), Ok(OpenTarget::Message(123)));
        }
        assert_eq!(
            parse_target("18446744073709551615", BASE),
            Ok(OpenTarget::Message(u64::MAX))
        );
    }

    #[test]
    fn accepts_thread_links_and_only_po_thread_links() {
        for path in ["/t/123", "/t/123/", "/t/123.html", "/Forum/po/id/123.html"] {
            let url = format!("https://www.nmbxd1.com{path}?page=2");
            assert_eq!(parse_target(&url, BASE), Ok(OpenTarget::Thread(123)));
        }
    }

    #[test]
    fn official_domains_and_configured_host_are_accepted() {
        for host in [
            "nmbxd.com",
            "www.nmbxd1.com",
            "nmbxd2.com",
            "nmbxd3.com",
            "nmb.best",
            "adnmb.com",
            "www.adnmb1.com",
            "adnmb2.com",
            "island.example",
        ] {
            assert_eq!(
                parse_target(&format!("https://{host}/t/9"), "https://island.example/"),
                Ok(OpenTarget::Thread(9))
            );
        }
    }

    #[test]
    fn reply_fragments_and_website_reply_queries_select_the_message() {
        for fragment in ["r456", "reply456", "reply_456", "post_456", "No.456", "456"] {
            assert_eq!(
                parse_target(&format!("{BASE}t/123#{fragment}"), BASE),
                Ok(OpenTarget::Message(456))
            );
        }
        assert_eq!(
            parse_target(&format!("{BASE}t/123?page=2&r=456"), BASE),
            Ok(OpenTarget::Message(456))
        );
        assert_eq!(
            parse_target(&format!("{BASE}t/123?r=456#r789"), BASE),
            Ok(OpenTarget::Message(789))
        );
        assert_eq!(
            parse_target(&format!("{BASE}t/123?r=0#heading"), BASE),
            Ok(OpenTarget::Thread(123))
        );
    }

    #[test]
    fn rejects_invalid_ids_and_unsupported_paths() {
        for input in [
            "",
            "0",
            "No.0",
            "-1",
            "+123",
            "18446744073709551616",
            "No.1 2",
            "text 123",
        ] {
            assert!(parse_target(input, BASE).is_err(), "{input}");
        }
        for path in [
            "/t/0",
            "/t/18446744073709551616",
            "/t/12x",
            "/t/123/other",
            "/ref/123",
            "/f/123",
            "/t/123.html.html",
        ] {
            assert!(
                parse_target(&format!("{BASE}{}", &path[1..]), BASE).is_err(),
                "{path}"
            );
        }
    }

    #[test]
    fn rejects_other_hosts_credentials_schemes_and_malformed_urls() {
        for input in [
            "https://evil.test/t/123",
            "https://nmbxd1.com.evil.test/t/123",
            "https://evilnmbxd1.com/t/123",
            "https://user@www.nmbxd1.com/t/123",
            "https://user:password@www.nmbxd1.com/t/123",
            "ftp://www.nmbxd1.com/t/123",
            "javascript:123",
            "/t/123",
            "https://www.nmbxd1.com/t/1\n23",
            "https://www.nmbxd1.com\\@evil.test/t/123",
        ] {
            assert!(parse_target(input, BASE).is_err(), "{input}");
        }
    }

    #[test]
    fn generated_links_use_configured_website_and_roundtrip() {
        for base in [
            BASE,
            "https://island.example:8443/",
            "https://island.example",
        ] {
            assert_eq!(
                parse_target(&thread_url(base, 123), base),
                Ok(OpenTarget::Thread(123))
            );
            assert_eq!(
                parse_target(&message_url(base, 456), base),
                Ok(OpenTarget::Message(456))
            );
        }
        assert_eq!(thread_url(BASE, 123), "https://www.nmbxd1.com/t/123");
        assert_eq!(message_url(BASE, 456), "rdnmb://message/456");
    }

    #[test]
    fn app_message_links_validate_the_whole_url() {
        for input in [
            "rdnmb://message/0",
            "rdnmb://message/18446744073709551616",
            "rdnmb://message/123/other",
            "rdnmb://message/123?r=456",
            "rdnmb://message/123#r456",
            "rdnmb://user@message/123",
            "rdnmb://other/123",
            "rdnmb://message:123/456",
        ] {
            assert!(parse_target(input, BASE).is_err(), "{input}");
        }
    }

    #[test]
    fn website_reply_links_retain_parent_context() {
        assert_eq!(
            enclosing_thread(&format!("{BASE}t/123?r=456"), BASE),
            Some(123)
        );
        assert_eq!(
            enclosing_thread(&format!("{BASE}t/123.html#r456"), BASE),
            Some(123)
        );
        assert_eq!(
            enclosing_thread(
                "https://island.example/t/123#r456",
                "https://island.example/"
            ),
            Some(123)
        );
        for input in [
            "123",
            "No.123",
            "rdnmb://message/123",
            "https://evil.test/t/123#r456",
            "https://www.nmbxd1.com/t/1\n23",
        ] {
            assert_eq!(enclosing_thread(input, BASE), None, "{input}");
        }
    }
}
