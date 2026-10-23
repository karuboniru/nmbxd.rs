//! Native image clipboard support; GPUI 0.2.2 advertises text only on Linux.

use arboard::{Clipboard, ImageData};
use gpui::{Image, ImageFormat};
use std::{
    borrow::Cow,
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

// On Linux the selection owner serves future paste requests. Keep it alive
// after the worker returns, and serialize operations across background workers.
// Clipboard is Send; the mutex also avoids simultaneous Windows clipboard opens.
static CLIPBOARD: OnceLock<Mutex<Option<Clipboard>>> = OnceLock::new();
static SHUTTING_DOWN: AtomicBool = AtomicBool::new(false);
const MAX_ENCODED_BYTES: usize = 65 * 1024 * 1024;

/// Call on a worker: bounded decode and native clipboard operations can block.
/// Successful writes publish image/png on Linux, NSImage on macOS, and a DIB on
/// Windows. Arboard tries Wayland data-control before falling back to X11.
pub fn write(image: Image) -> Result<(), String> {
    let pixels = image_data(image)?;
    let mut owner = CLIPBOARD
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| "图片剪贴板暂时不可用，请重启应用后重试".to_owned())?;
    if SHUTTING_DOWN.load(Ordering::Acquire) {
        return Err("应用正在退出，无法复制图片".to_owned());
    }
    if owner.is_none() {
        *owner = Some(Clipboard::new().map_err(clipboard_error)?);
    }
    owner
        .as_mut()
        .unwrap()
        .set_image(pixels)
        .map_err(clipboard_error)
}

fn image_data(image: Image) -> Result<ImageData<'static>, String> {
    if image.format() != ImageFormat::Png {
        return Err("剪贴板图片必须为 PNG 格式".to_owned());
    }
    if image.bytes().len() > MAX_ENCODED_BYTES {
        return Err("剪贴板图片超过大小限制".to_owned());
    }
    let (pixels, format) = crate::media::decode_rgba(image.bytes())?;
    if format != image::ImageFormat::Png {
        return Err("剪贴板图片必须为 PNG 格式".to_owned());
    }
    Ok(ImageData {
        width: pixels.width() as usize,
        height: pixels.height() as usize,
        bytes: Cow::Owned(pixels.into_raw()),
    })
}

fn clipboard_error(error: arboard::Error) -> String {
    match error {
        arboard::Error::ClipboardOccupied => "系统剪贴板正忙，请稍后重试".to_owned(),
        arboard::Error::ConversionFailure => "无法转换系统剪贴板图片".to_owned(),
        _ => {
            #[cfg(target_os = "linux")]
            {
                "无法写入系统图片剪贴板；Wayland 需要 data-control 支持或可用的 XWayland（DISPLAY）"
                    .to_owned()
            }
            #[cfg(not(target_os = "linux"))]
            {
                "无法写入系统图片剪贴板，请稍后重试".to_owned()
            }
        }
    }
}

/// Release the native selection owner when the application is quitting.
/// Linux clipboard persistence after application exit depends on the desktop's
/// clipboard manager. Call on a worker; release can wait for an active write.
pub fn shutdown() {
    SHUTTING_DOWN.store(true, Ordering::Release);
    if let Some(owner) = CLIPBOARD.get() {
        let mut owner = owner.lock().unwrap_or_else(|error| error.into_inner());
        owner.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn native_image_data_preserves_rgba_order_and_dimensions() {
        let pixels = image::RgbaImage::from_pixel(2, 1, image::Rgba([250, 20, 40, 128]));
        let mut bytes = Cursor::new(Vec::new());
        pixels
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let data = image_data(Image::from_bytes(ImageFormat::Png, bytes.into_inner())).unwrap();
        assert_eq!((data.width, data.height), (2, 1));
        assert_eq!(&*data.bytes, &[250, 20, 40, 128, 250, 20, 40, 128]);
    }

    #[test]
    fn rejects_other_formats_and_mislabeled_data() {
        assert!(image_data(Image::from_bytes(ImageFormat::Jpeg, Vec::new())).is_err());
        let mut jpeg = Cursor::new(Vec::new());
        image::RgbImage::new(1, 1)
            .write_to(&mut jpeg, image::ImageFormat::Jpeg)
            .unwrap();
        assert!(image_data(Image::from_bytes(ImageFormat::Png, jpeg.into_inner())).is_err());
    }

    #[test]
    fn rejects_oversized_encoded_clipboard_data_before_decoding() {
        let image = Image::from_bytes(ImageFormat::Png, vec![0; MAX_ENCODED_BYTES + 1]);
        assert!(image_data(image).unwrap_err().contains("大小限制"));
    }
}
