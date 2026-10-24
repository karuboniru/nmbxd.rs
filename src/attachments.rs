//! Local attachment selection and bounded decoding. No uploads happen here.
use gpui::{ClipboardEntry, ClipboardItem, Image, RenderImage, Window};
use image::{DynamicImage, Frame, ImageDecoder, ImageEncoder, ImageReader, Limits};
use rdnmb::api::{ImageAttachment, MAX_IMAGE_BYTES};
use std::{
    fs::File,
    future::Future,
    io::{Cursor, Read, Write},
    path::PathBuf,
    sync::Arc,
};

const MAX_DIMENSION: u32 = 8192;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_DECODE_BYTES: u64 = 64 * 1024 * 1024;
const PREVIEW_DIMENSION: u32 = 640;

#[derive(Clone)]
pub struct DraftImage {
    pub attachment: ImageAttachment,
    pub preview: Arc<RenderImage>,
}

/// Construct and poll on the UI thread; decode the selected file on a worker.
/// Linux talks directly to the portal so errors never launch an external picker.
#[cfg(target_os = "linux")]
pub fn pick_image(
    window: &Window,
) -> impl Future<Output = Result<Option<PathBuf>, String>> + 'static + use<> {
    use ashpd::{
        WindowIdentifier,
        desktop::{
            ResponseError,
            file_chooser::{FileFilter, OpenFileRequest},
        },
    };
    use raw_window_handle::{HasDisplayHandle, HasWindowHandle};

    let window_handle = HasWindowHandle::window_handle(window).map(|handle| handle.as_raw());
    let display_handle = HasDisplayHandle::display_handle(window).map(|handle| handle.as_raw());
    async move {
        let window_handle = window_handle.map_err(|_| "无法获取图片选择窗口".to_owned())?;
        let display_handle = display_handle.map_err(|_| "无法获取图片选择显示器".to_owned())?;
        let identifier =
            WindowIdentifier::from_raw_handle(&window_handle, Some(&display_handle)).await;
        let result = OpenFileRequest::default()
            .identifier(identifier)
            .title("选择要附加的图片")
            .modal(true)
            .multiple(false)
            .filters([FileFilter::new("图片 (JPEG、PNG、GIF)")
                .mimetype("image/jpeg")
                .mimetype("image/png")
                .mimetype("image/gif")])
            .send()
            .await
            .and_then(|request| request.response());
        match result {
            Ok(response) => response.uris().first().map_or(Ok(None), |uri| {
                uri.to_file_path()
                    .map(Some)
                    .map_err(|_| "所选图片无法作为本地文件读取".to_owned())
            }),
            Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(None),
            Err(_) => Err("无法打开系统图片选择器，请确认已安装并启用 xdg-desktop-portal 及桌面对应的文件选择后端".to_owned()),
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub fn pick_image(
    window: &Window,
) -> impl Future<Output = Result<Option<PathBuf>, String>> + 'static + use<> {
    // Construct the dialog on the UI thread, as required by macOS.
    let picker = crate::file_dialog::for_window(window)
        .set_title("选择要附加的图片")
        .add_filter("图片 (JPEG、PNG、GIF)", &["jpg", "jpeg", "png", "gif"])
        .pick_file();
    async move { Ok(picker.await.map(|file| file.path().to_owned())) }
}

/// Move the clipboard image to a worker without copying its encoded bytes.
pub fn clipboard_image(item: ClipboardItem) -> Option<Image> {
    item.into_entries().find_map(|entry| match entry {
        ClipboardEntry::Image(image) => Some(image),
        ClipboardEntry::String(_) => None,
    })
}

/// Run on a background worker: read at most the upload cap plus one byte.
pub fn load_file(path: PathBuf) -> Result<DraftImage, String> {
    let file = File::open(&path).map_err(|_| "无法读取所选图片，请重新选择".to_owned())?;
    let metadata = file
        .metadata()
        .map_err(|_| "无法读取图片文件信息".to_owned())?;
    if !metadata.is_file() {
        return Err("请选择图片文件".to_owned());
    }
    if metadata.len() > MAX_IMAGE_BYTES as u64 {
        return Err(size_error());
    }
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "图片文件读取失败".to_owned())?;
    check_size(&bytes)?;
    let (decoded, format) = decode(&bytes)?;
    if !upload_format(format) {
        return Err("请选择 JPEG、PNG 或 GIF 图片".to_owned());
    }
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image");
    draft(filename, bytes, decoded)
}

/// GPUI 0.2.2 clipboard images contain encoded bytes. Preserve JPEG/PNG/GIF;
/// convert WebP/BMP/TIFF to a bounded PNG for the posting API.
pub fn load_clipboard(image: Image) -> Result<DraftImage, String> {
    check_size(image.bytes())?;
    let (decoded, format) = decode(image.bytes())?;
    if upload_format(format) {
        let extension = match format {
            image::ImageFormat::Jpeg => "jpg",
            image::ImageFormat::Gif => "gif",
            _ => "png",
        };
        draft(&format!("clipboard.{extension}"), image.bytes, decoded)
    } else {
        let rgba = decoded.to_rgba8();
        let mut output = CappedWriter(Vec::new());
        image::codecs::png::PngEncoder::new(&mut output)
            .write_image(
                &rgba,
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|_| "剪贴板图片转换失败或转换后超过 10 MiB 限制".to_owned())?;
        draft("clipboard.png", output.0, decoded)
    }
}

fn draft(filename: &str, bytes: Vec<u8>, decoded: DynamicImage) -> Result<DraftImage, String> {
    let attachment = ImageAttachment::new(filename, bytes).map_err(|error| error.to_string())?;
    let mut pixels = if decoded.width() > PREVIEW_DIMENSION || decoded.height() > PREVIEW_DIMENSION
    {
        decoded.thumbnail(PREVIEW_DIMENSION, PREVIEW_DIMENSION)
    } else {
        decoded
    }
    .into_rgba8();
    // RenderImage's image buffers contain BGRA, unlike image's RGBA output.
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok(DraftImage {
        attachment,
        preview: Arc::new(RenderImage::new(vec![Frame::new(pixels)])),
    })
}

fn decode(bytes: &[u8]) -> Result<(DynamicImage, image::ImageFormat), String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "无法识别图片格式".to_owned())?;
    let format = reader
        .format()
        .ok_or_else(|| "无法识别图片格式".to_owned())?;
    if !matches!(
        format,
        image::ImageFormat::Jpeg
            | image::ImageFormat::Png
            | image::ImageFormat::Gif
            | image::ImageFormat::WebP
            | image::ImageFormat::Bmp
            | image::ImageFormat::Tiff
    ) {
        return Err("暂不支持此剪贴板图片格式，请使用 PNG、JPEG 或 GIF".to_owned());
    }
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODE_BYTES);
    reader.limits(limits.clone());
    let mut decoder = reader
        .into_decoder()
        .map_err(|_| "图片损坏或尺寸超过限制".to_owned())?;
    let (width, height) = decoder.dimensions();
    if width == 0
        || height == 0
        || width > MAX_DIMENSION
        || height > MAX_DIMENSION
        || u64::from(width) * u64::from(height) > MAX_PIXELS
        || decoder.total_bytes() > MAX_DECODE_BYTES
    {
        return Err("图片尺寸超过限制（单边最大 8192 像素，最多 1600 万像素）".to_owned());
    }
    decoder
        .set_limits(limits)
        .map_err(|_| "图片尺寸超过限制".to_owned())?;
    DynamicImage::from_decoder(decoder)
        .map(|image| (image, format))
        .map_err(|_| "图片解码失败".to_owned())
}

fn upload_format(format: image::ImageFormat) -> bool {
    matches!(
        format,
        image::ImageFormat::Jpeg | image::ImageFormat::Png | image::ImageFormat::Gif
    )
}

fn check_size(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() > MAX_IMAGE_BYTES {
        Err(size_error())
    } else {
        Ok(())
    }
}

fn size_error() -> String {
    "图片文件超过 10 MiB 限制".to_owned()
}

struct CappedWriter(Vec<u8>);

impl Write for CappedWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > MAX_IMAGE_BYTES.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("attachment exceeds size limit"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let pixels = image::RgbaImage::from_pixel(width, height, image::Rgba([250, 20, 40, 255]));
        let mut encoded = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(pixels)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        encoded.into_inner()
    }

    #[test]
    fn clipboard_preserves_upload_and_bgra_preview() {
        let bytes = png(2, 1);
        let draft =
            load_clipboard(Image::from_bytes(gpui::ImageFormat::Png, bytes.clone())).unwrap();
        assert_eq!(draft.attachment.bytes(), bytes);
        assert_eq!(
            draft.preview.as_bytes(0).unwrap(),
            &[40, 20, 250, 255, 40, 20, 250, 255]
        );
    }

    #[test]
    fn rejects_large_dimensions_before_decode() {
        assert!(decode(&png(MAX_DIMENSION + 1, 1)).is_err());
        assert!(decode(b"not an image").is_err());
    }

    #[test]
    fn encoded_png_output_cannot_exceed_upload_cap() {
        let mut writer = CappedWriter(vec![0; MAX_IMAGE_BYTES - 1]);
        assert!(writer.write_all(&[1, 2]).is_err());
        assert_eq!(writer.0.len(), MAX_IMAGE_BYTES - 1);
    }
}
