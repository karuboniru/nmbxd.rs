//! Original image exports. Network, decoding, encoding, and writes run on workers.

use gpui::{Image, ImageFormat, Window};
use std::{
    fs::OpenOptions,
    future::Future,
    io::{self, Write},
    path::PathBuf,
};

pub struct ExportImage {
    /// The unmodified response, preserving animation and the source format.
    pub bytes: Vec<u8>,
    pub filename: String,
    /// PNG uses RGBA (unlike the viewer's BGRA buffer) and the first frame.
    pub clipboard: Image,
}

/// Run on a background worker. Uses the viewer's origin and resource limits.
pub fn fetch(url: &str) -> Result<ExportImage, String> {
    let bytes = crate::media::fetch_bytes(url)?;
    prepare(url, bytes)
}

fn prepare(url: &str, bytes: Vec<u8>) -> Result<ExportImage, String> {
    let url = crate::media::validate_url(url)?;
    let (pixels, format) = crate::media::decode_rgba(&bytes)?;
    let mut png = CappedPng(Vec::new());
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut png),
        pixels.as_raw(),
        pixels.width(),
        pixels.height(),
        image::ExtendedColorType::Rgba8,
    )
    .map_err(|_| "无法生成剪贴板图片".to_owned())?;
    let extension = match format {
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::Png => "png",
        image::ImageFormat::Gif => "gif",
        image::ImageFormat::WebP => "webp",
        _ => return Err("暂不支持此图片格式".to_owned()),
    };
    let name = url.path().rsplit('/').next().unwrap_or("image");
    let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
    let stem: String = stem
        .chars()
        .take(100)
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let stem = if stem.is_empty() { "image" } else { &stem };
    Ok(ExportImage {
        bytes,
        filename: format!("{stem}.{extension}"),
        clipboard: Image::from_bytes(ImageFormat::Png, png.0),
    })
}

// Prevent encoded clipboard data from growing without a bound. RGBA pixels are
// limited to 64 MiB; reserve a little extra for PNG framing overhead.
struct CappedPng(Vec<u8>);

impl Write for CappedPng {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.0.len().saturating_add(bytes.len()) > 65 * 1024 * 1024 {
            return Err(io::Error::other("clipboard image exceeds size limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Construct and poll on the UI thread. Cancellation returns `Ok(None)`.
/// A portal error is reported directly, with no external picker fallback.
#[cfg(target_os = "linux")]
pub fn pick_save(
    window: &Window,
    filename: String,
) -> impl Future<Output = Result<Option<PathBuf>, String>> + 'static + use<> {
    use ashpd::{
        WindowIdentifier,
        desktop::{ResponseError, file_chooser::SaveFileRequest},
    };
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

    let window_handle = HasWindowHandle::window_handle(window).map(|handle| handle.as_raw());
    let display_handle = HasDisplayHandle::display_handle(window).map(|handle| handle.as_raw());
    async move {
        let window_handle = window_handle.map_err(|_| "无法获取图片保存窗口".to_owned())?;
        let display_handle = display_handle.map_err(|_| "无法获取图片保存显示器".to_owned())?;
        let identifier =
            WindowIdentifier::from_raw_handle(&window_handle, Some(&display_handle)).await;
        let response = SaveFileRequest::default()
            .identifier(identifier)
            .title("保存图片")
            .accept_label("保存")
            .current_name(filename.as_str())
            .modal(true)
            .send()
            .await
            .and_then(|request| request.response());
        match response {
            Ok(response) => selection_path(response.uris().first()),
            Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(None),
            Err(_) => Err("无法打开系统保存对话框，请确认已安装并启用 xdg-desktop-portal 及桌面对应的文件选择后端".to_owned()),
        }
    }
}

#[cfg(target_os = "linux")]
fn selection_path(uri: Option<&reqwest::Url>) -> Result<Option<PathBuf>, String> {
    uri.map(|uri| {
        uri.to_file_path()
            .map_err(|_| "请选择本地保存路径".to_owned())
    })
    .transpose()
}

#[cfg(not(target_os = "linux"))]
pub fn pick_save(
    window: &Window,
    filename: String,
) -> impl Future<Output = Result<Option<PathBuf>, String>> + 'static + use<> {
    let picker = rfd::AsyncFileDialog::new()
        .set_parent(window)
        .set_title("保存图片")
        .set_file_name(filename)
        .save_file();
    async move { Ok(picker.await.map(|file| file.path().to_owned())) }
}

/// Run on a worker. Atomic create-new prevents overwrites and rejects a final
/// symlink, including dangling links. Existing names must be changed in picker.
pub fn save(path: PathBuf, bytes: Vec<u8>) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                "文件已存在，请选择其他保存名称".to_owned()
            } else {
                "无法创建图片文件，请检查保存路径和权限".to_owned()
            }
        })?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| "图片写入失败，目标文件可能不完整，请检查剩余空间".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn source_image() -> Vec<u8> {
        let mut bytes = Cursor::new(Vec::new());
        image::RgbaImage::from_pixel(2, 1, image::Rgba([250, 20, 40, 255]))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    #[test]
    fn clipboard_png_preserves_rgba_and_original_bytes() {
        let bytes = source_image();
        let result = prepare("https://image.nmb.best/image/example.jpg", bytes.clone()).unwrap();
        assert_eq!(result.bytes, bytes);
        assert_eq!(result.filename, "example.png");
        let decoded = image::load_from_memory(result.clipboard.bytes()).unwrap();
        assert_eq!(decoded.into_rgba8().get_pixel(0, 0).0, [250, 20, 40, 255]);
    }

    #[test]
    fn refuses_untrusted_image_origins_before_fetching() {
        assert!(fetch("https://example.com/image/example.png").is_err());
        assert!(fetch("https://image.nmb.best/image/example.png?token=x").is_err());
    }

    #[test]
    fn animated_gif_stays_original_while_clipboard_uses_first_frame() {
        let red = image::RgbaImage::from_pixel(1, 1, image::Rgba([250, 20, 40, 255]));
        let blue = image::RgbaImage::from_pixel(1, 1, image::Rgba([10, 30, 240, 255]));
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            encoder.encode_frame(image::Frame::new(red)).unwrap();
            encoder.encode_frame(image::Frame::new(blue)).unwrap();
        }
        let result = prepare("https://image.nmb.best/image/animation.gif", bytes.clone()).unwrap();
        assert_eq!(result.bytes, bytes);
        assert_eq!(result.filename, "animation.gif");
        let decoded = image::load_from_memory(result.clipboard.bytes()).unwrap();
        assert_eq!(decoded.into_rgba8().get_pixel(0, 0).0, [250, 20, 40, 255]);
        let path = std::env::temp_dir().join(format!("rdnmb-export-{}", uuid::Uuid::new_v4()));
        save(path.clone(), result.bytes).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn saves_original_bytes_without_overwriting_existing_file() {
        let path = std::env::temp_dir().join(format!("rdnmb-export-{}", uuid::Uuid::new_v4()));
        let bytes = source_image();
        save(path.clone(), bytes.clone()).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(save(path.clone(), b"replacement".to_vec()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn refuses_dangling_symlink_destinations() {
        let path = std::env::temp_dir().join(format!("rdnmb-export-{}", uuid::Uuid::new_v4()));
        let target = path.with_extension("target");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(save(path.clone(), b"image".to_vec()).is_err());
        assert!(!target.exists());
        std::fs::remove_file(path).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn empty_save_selection_is_cancellation() {
        assert_eq!(selection_path(None).unwrap(), None);
        let remote = reqwest::Url::parse("https://example.com/image.png").unwrap();
        assert!(selection_path(Some(&remote)).is_err());
    }
}
