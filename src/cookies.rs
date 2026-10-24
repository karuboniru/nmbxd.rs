//! Named credentials. Values and import URLs must never appear in diagnostics.
use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose};
use serde::Deserialize;
use std::{collections::BTreeMap, fs, io::Write, path::Path};

const MAX_COOKIE_BYTES: usize = 8192;

pub fn validate_name(name: &str) -> Result<()> {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    if name.is_empty()
        || name.len() > 120
        || name.trim() != name
        || name.starts_with('.')
        || name.ends_with('.')
        || name
            .chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
        || [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
            "COM8", "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
        ]
        .contains(&stem.as_str())
    {
        bail!("饼干名称无效：请使用不含路径或特殊字符的名称（最多 120 字节）");
    }
    Ok(())
}

pub fn validate_cookie(cookie: &str) -> Result<()> {
    // Validate the whole input before extracting userhash; an otherwise unused
    // part of a pasted header must not conceal a line break or control byte.
    if cookie.is_empty()
        || cookie.len() > MAX_COOKIE_BYTES
        || !cookie.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
    {
        bail!("饼干内容无效：必须是非空的 HTTP Cookie，且不含换行或控制字符");
    }
    let value = cookie
        .split(';')
        .find_map(|part| part.trim().strip_prefix("userhash="))
        .unwrap_or(cookie)
        .trim();
    if value.is_empty() || value.contains(';') {
        bail!("饼干格式无效：请提供 userhash 值或含 userhash 的 Cookie");
    }
    Ok(())
}

fn decode_query_component(value: &str) -> Result<String> {
    let mut bytes = Vec::with_capacity(value.len());
    let mut input = value.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let hi = input.next().and_then(|b| (b as char).to_digit(16));
            let lo = input.next().and_then(|b| (b as char).to_digit(16));
            let (Some(hi), Some(lo)) = (hi, lo) else {
                bail!("二维码链接包含无效的百分号编码");
            };
            bytes.push((hi * 16 + lo) as u8);
        } else {
            // Standard base64 uses '+'. Query form decoding would corrupt it.
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("二维码链接参数编码无效"))
}

/// Decode a QR image URL locally. Never fetch the URL or expose it in errors.
pub fn parse_import_url(input: &str) -> Result<(String, String)> {
    if input.len() > 32_768 {
        bail!("二维码链接过长");
    }
    let url = reqwest::Url::parse(input.trim())
        .map_err(|_| anyhow::anyhow!("请输入有效的二维码图片链接"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str() != Some("publicapi.ovear.info")
        || url.path() != "/qrcode.php"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        bail!("请粘贴 publicapi.ovear.info 的 HTTP 或 HTTPS 二维码图片链接");
    }
    let mut payload = None;
    let mut base64_flag = None;
    for parameter in url.query().unwrap_or_default().split('&') {
        let (key, value) = parameter.split_once('=').unwrap_or((parameter, ""));
        let key = decode_query_component(key)?;
        if key == "text" || key == "base64" {
            let slot = if key == "text" {
                &mut payload
            } else {
                &mut base64_flag
            };
            if slot.is_some() {
                bail!("二维码链接含重复参数");
            }
            *slot = Some(decode_query_component(value)?);
        }
    }
    if base64_flag.as_deref() != Some("1") {
        bail!("二维码链接需要 base64=1 参数");
    }
    let payload = payload.context("二维码链接缺少 text 参数")?;
    let bytes = general_purpose::STANDARD
        .decode(&payload)
        .or_else(|_| general_purpose::STANDARD_NO_PAD.decode(&payload))
        .map_err(|_| anyhow::anyhow!("二维码内容不是有效的 Base64"))?;
    #[derive(Deserialize)]
    struct ImportedCookie {
        name: String,
        cookie: String,
    }
    let imported: ImportedCookie = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("二维码内容需要包含 name 和 cookie 字符串"))?;
    validate_name(&imported.name)?;
    validate_cookie(&imported.cookie)?;
    Ok((imported.name, imported.cookie))
}

fn restrict_permissions(path: &Path, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))
            .context("无法限制饼干文件权限")?;
    }
    #[cfg(not(unix))]
    let _ = (path, mode);
    Ok(())
}

fn check_directory(directory: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(directory).context("无法读取饼干目录")?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        bail!("饼干目录必须是普通目录");
    }
    restrict_permissions(directory, 0o700)
}

fn read_cookie(path: &Path) -> Result<String> {
    let metadata = fs::symlink_metadata(path).context("无法读取饼干文件")?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_COOKIE_BYTES as u64
    {
        bail!("饼干文件必须是大小有效的普通文件");
    }
    restrict_permissions(path, 0o600)?;
    let cookie = fs::read_to_string(path).context("无法读取饼干文件")?;
    validate_cookie(&cookie)?;
    Ok(cookie)
}

pub fn load_from(directory: &Path) -> Result<BTreeMap<String, String>> {
    match fs::symlink_metadata(directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(error) => return Err(error).context("无法读取饼干目录"),
        Ok(_) => check_directory(directory)?,
    }
    let mut cookies = BTreeMap::new();
    for entry in fs::read_dir(directory).context("无法读取饼干目录")? {
        let entry = entry.context("无法读取饼干目录项目")?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("cookie") {
            continue;
        }
        let name = path
            .file_stem()
            .and_then(|name| name.to_str())
            .context("饼干文件名无效")?;
        validate_name(name)?;
        cookies.insert(name.to_owned(), read_cookie(&path)?);
    }
    Ok(cookies)
}

/// Idempotent insertion. A different value at an existing name is never replaced.
pub fn store_new_in(directory: &Path, name: &str, cookie: &str) -> Result<()> {
    store_new_tracked_in(directory, name, cookie).map(|_| ())
}

/// Return whether this call created the file, for settings-save rollback.
pub(crate) fn store_new_tracked_in(directory: &Path, name: &str, cookie: &str) -> Result<bool> {
    validate_name(name)?;
    validate_cookie(cookie)?;
    fs::create_dir_all(directory).context("无法创建饼干目录")?;
    check_directory(directory)?;
    let path = directory.join(format!("{name}.cookie"));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(&path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if read_cookie(&path)? == cookie {
                return Ok(false);
            }
            bail!("同名饼干已存在；请使用其他名称，现有文件未被覆盖");
        }
        Err(error) => return Err(error).context("无法创建饼干文件"),
    };
    let result = (|| {
        file.write_all(cookie.as_bytes())
            .context("无法写入饼干文件")?;
        file.sync_all().context("无法同步饼干文件")
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result.map(|_| true)
}

/// Only explicitly selected, unchanged credentials may be removed. An absent
/// file is already removed; a symlink or a replacement is always a conflict.
pub fn check_removal_in(directory: &Path, name: &str, expected: &str) -> Result<bool> {
    validate_name(name)?;
    validate_cookie(expected)?;
    match fs::symlink_metadata(directory) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("无法读取饼干目录"),
        Ok(_) => check_directory(directory)?,
    }
    let path = directory.join(format!("{name}.cookie"));
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).context("无法读取待删除的饼干文件"),
        Ok(_) => {}
    }
    if read_cookie(&path)? != expected {
        bail!("待删除的饼干文件已发生变化；请重启客户端后重试，现有文件未被删除");
    }
    Ok(true)
}

/// Recheck immediately before deletion so edits made while preparing a save
/// cannot silently delete a different credential.
pub fn remove_if_matches_in(directory: &Path, name: &str, expected: &str) -> Result<bool> {
    if !check_removal_in(directory, name, expected)? {
        return Ok(false);
    }
    // Move first, then verify the exact file being removed. A concurrent rename
    // at the original path cannot cause a replacement credential to be unlinked.
    let staging = loop {
        let staging = directory.join(format!(".delete-{}", uuid::Uuid::new_v4()));
        match fs::create_dir(&staging) {
            Ok(()) => break staging,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error).context("无法准备删除饼干文件"),
        }
    };
    if let Err(error) = restrict_permissions(&staging, 0o700) {
        let _ = fs::remove_dir(&staging);
        return Err(error);
    }
    let path = directory.join(format!("{name}.cookie"));
    let staged = staging.join("removed.cookie");
    if let Err(error) = fs::rename(&path, &staged) {
        let _ = fs::remove_dir(&staging);
        return if error.kind() == std::io::ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(error).context("无法删除饼干文件")
        };
    }
    let result = (|| {
        check_removal_in(&staging, "removed", expected)?;
        fs::remove_file(&staged).context("无法删除饼干文件")?;
        Ok(true)
    })();
    if result.is_err() {
        // hard_link creates the original name only if absent. Never overwrite
        // a new file that appeared while the deletion was being validated.
        if fs::hard_link(&staged, &path).is_err() {
            return result.context(
                "删除失败，饼干文件已保留在 cookie 目录的 .delete-* 子目录中，请检查文件后重试",
            );
        }
        let _ = fs::remove_file(&staged);
    }
    let _ = fs::remove_dir(&staging);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removals_validate_names_contents_and_leave_other_credentials_alone() {
        let directory =
            std::env::temp_dir().join(format!("rdnmb-remove-cookies-{}", uuid::Uuid::new_v4()));
        store_new_in(&directory, "saved", "original-secret").unwrap();
        store_new_in(&directory, "other", "other-secret").unwrap();
        assert!(remove_if_matches_in(&directory, "../other", "other-secret").is_err());
        let error = remove_if_matches_in(&directory, "saved", "wrong-secret").unwrap_err();
        assert!(!format!("{error:#}").contains("secret"));
        assert_eq!(load_from(&directory).unwrap()["saved"], "original-secret");
        assert!(remove_if_matches_in(&directory, "saved", "original-secret").unwrap());
        assert!(!remove_if_matches_in(&directory, "saved", "original-secret").unwrap());
        assert_eq!(load_from(&directory).unwrap()["other"], "other-secret");
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn removal_rejects_symlinks_and_directories() {
        let directory =
            std::env::temp_dir().join(format!("rdnmb-remove-links-{}", uuid::Uuid::new_v4()));
        store_new_in(&directory, "target", "target-secret").unwrap();
        std::os::unix::fs::symlink(
            directory.join("target.cookie"),
            directory.join("link.cookie"),
        )
        .unwrap();
        assert!(remove_if_matches_in(&directory, "link", "target-secret").is_err());
        assert!(
            fs::symlink_metadata(directory.join("link.cookie"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(directory.join("target.cookie")).unwrap(),
            "target-secret"
        );
        fs::create_dir(directory.join("folder.cookie")).unwrap();
        assert!(remove_if_matches_in(&directory, "folder", "target-secret").is_err());
        assert!(directory.join("folder.cookie").is_dir());
        fs::remove_dir_all(directory).unwrap();
    }

    fn url(name: &str, cookie: &str) -> String {
        let json = serde_json::json!({"name":name,"cookie":cookie});
        format!(
            "https://publicapi.ovear.info/qrcode.php?w=500&text={}&base64=1",
            general_purpose::STANDARD.encode(json.to_string())
        )
    }

    #[test]
    fn parses_raw_and_percent_encoded_base64_plus_and_query_order() {
        // U+083E ensures a '+' in UTF-8 JSON's standard base64 encoding.
        let raw = url("࠾", "userhash=test-only");
        assert!(raw.contains('+'));
        assert_eq!(
            parse_import_url(&raw).unwrap(),
            ("࠾".into(), "userhash=test-only".into())
        );
        let encoded = raw.replace('+', "%2B").replace("text=", "%74ext=");
        assert_eq!(parse_import_url(&encoded).unwrap().0, "࠾");
        let payload = raw
            .split("text=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        assert!(
            parse_import_url(&format!(
                "https://publicapi.ovear.info/qrcode.php?base64=1&text={payload}&w=500"
            ))
            .is_ok()
        );
    }

    #[test]
    fn accepts_http_and_https_with_their_default_ports() {
        let original = url("test", "%09%AB");
        for origin in [
            "http://publicapi.ovear.info",
            "http://publicapi.ovear.info:80",
            "https://publicapi.ovear.info",
            "https://publicapi.ovear.info:443",
        ] {
            let input = original.replace("https://publicapi.ovear.info", origin);
            assert_eq!(
                parse_import_url(&input).unwrap(),
                ("test".into(), "%09%AB".into())
            );
        }
        for origin in [
            "ftp://publicapi.ovear.info",
            "http://publicapi.ovear.info:443",
            "https://publicapi.ovear.info:80",
        ] {
            assert!(
                parse_import_url(&original.replace("https://publicapi.ovear.info", origin))
                    .is_err()
            );
        }
    }

    #[test]
    fn rejects_invalid_urls_payloads_names_and_header_injection_without_secret_errors() {
        for input in [
            "https://example.com/",
            "http://publicapi.ovear.info/qrcode.php?text=e30=&base64=1",
            "https://publicapi.ovear.info/qrcode.php?text=%ZZ&base64=1",
            "https://publicapi.ovear.info/qrcode.php?text=e30=&text=e30=&base64=1",
            "https://publicapi.ovear.info/qrcode.php?text=e30=&base64=1",
        ] {
            assert!(parse_import_url(input).is_err());
        }
        for name in [
            "",
            ".",
            "..",
            "../outside",
            "a/b",
            "a\\b",
            "a:",
            " name",
            "name.",
            "NUL",
            "CON.txt",
            "a\n",
        ] {
            assert!(parse_import_url(&url(name, "secret-test")).is_err());
        }
        for cookie in [
            "",
            "userhash=",
            "foo=x;bar=y",
            "userhash=secret-test\r\nInjected: yes",
            "userhash=ok;\nsecret-test",
            "secret-test\0",
        ] {
            let message = parse_import_url(&url("safe", cookie))
                .unwrap_err()
                .to_string();
            assert!(!message.contains("secret-test"));
        }
    }

    #[test]
    fn stores_raw_credentials_without_overwrite_and_restricts_permissions() {
        let directory =
            std::env::temp_dir().join(format!("rdnmb-cookies-{}", uuid::Uuid::new_v4()));
        store_new_in(&directory, "我的饼干", "userhash=first").unwrap();
        store_new_in(&directory, "我的饼干", "userhash=first").unwrap();
        assert!(store_new_in(&directory, "我的饼干", "userhash=second").is_err());
        assert_eq!(load_from(&directory).unwrap()["我的饼干"], "userhash=first");
        assert_eq!(
            fs::read(directory.join("我的饼干.cookie")).unwrap(),
            b"userhash=first"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(directory.join("我的饼干.cookie"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            std::os::unix::fs::symlink(
                directory.join("我的饼干.cookie"),
                directory.join("link.cookie"),
            )
            .unwrap();
            assert!(load_from(&directory).is_err());
        }
        fs::remove_dir_all(directory).unwrap();
    }
}
