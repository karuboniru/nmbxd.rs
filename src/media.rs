//! On-demand, bounded image loading. Call `load` only for visible media.
//!
//! The store is a GPUI entity: observe it to repaint when requests finish. HTTP
//! and decoding run on background workers, independently of the API cookie.
//! Animated formats deliberately show only their first frame.

use gpui::{Context, RenderImage};
use image::{DynamicImage, Frame, ImageDecoder, ImageReader, Limits};
use reqwest::{Url, blocking::Client};
use std::{
    collections::{HashMap, VecDeque},
    io::{Cursor, Read},
    sync::{Arc, Condvar, Mutex, OnceLock},
    time::Duration,
};

const MAX_BODY_BYTES: usize = 12 * 1024 * 1024;
const MAX_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_DIMENSION: u32 = 8192;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;
const MAX_READY_ENTRIES: usize = 32;
const MAX_ENTRIES: usize = 64;
const MAX_CONCURRENT: usize = 2;
const MAX_QUEUED: usize = 32;

#[derive(Clone)]
pub enum MediaState {
    Loading,
    Ready(Arc<RenderImage>),
    Failed(String),
}

struct Entry {
    state: MediaState,
    bytes: usize,
    touched: u64,
}

#[derive(Default)]
pub struct MediaStore {
    entries: HashMap<String, Entry>,
    queued: VecDeque<String>,
    running: usize,
    cache_bytes: usize,
    clock: u64,
}

impl MediaStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read cached state without scheduling network or decode work.
    pub fn peek(&self, url: &str) -> Option<MediaState> {
        self.entries.get(url).map(|entry| entry.state.clone())
    }

    /// Request a visible image, or touch and return an existing cache entry.
    /// A full queue returns Loading; a subsequent paint can request it again.
    pub fn load(&mut self, url: &str, cx: &mut Context<Self>) -> MediaState {
        self.clock = self.clock.saturating_add(1);
        if let Some(entry) = self.entries.get_mut(url) {
            entry.touched = self.clock;
            return entry.state.clone();
        }
        if let Err(error) = validate_url(url) {
            return MediaState::Failed(error);
        }
        if self.queued.len() >= MAX_QUEUED {
            return MediaState::Loading;
        }
        // Pending entries are never evicted. Their count is bounded by the
        // queue plus workers, so an evictable entry exists at this limit.
        while self.entries.len() >= MAX_ENTRIES {
            if !self.evict_oldest() {
                return MediaState::Loading;
            }
        }
        self.entries.insert(
            url.to_owned(),
            Entry {
                state: MediaState::Loading,
                bytes: 0,
                touched: self.clock,
            },
        );
        self.queued.push_back(url.to_owned());
        self.pump(cx);
        MediaState::Loading
    }

    /// Failed requests remain failed until this explicit retry.
    pub fn retry(&mut self, url: &str, cx: &mut Context<Self>) -> MediaState {
        if matches!(self.peek(url), Some(MediaState::Failed(_))) {
            self.entries.remove(url);
        }
        let state = self.load(url, cx);
        cx.notify();
        state
    }

    fn pump(&mut self, cx: &mut Context<Self>) {
        while self.running < MAX_CONCURRENT {
            let Some(url) = self.queued.pop_front() else {
                break;
            };
            self.running += 1;
            let request_url = url.clone();
            let task = cx
                .background_executor()
                .spawn(async move { fetch_image(&request_url) });
            cx.spawn(async move |store, cx| {
                let result = task.await;
                let _ = store.update(cx, |store, cx| {
                    store.running -= 1;
                    if let Some(entry) = store.entries.get_mut(&url) {
                        match result {
                            Ok(image) => {
                                entry.bytes = image.as_bytes(0).map_or(0, <[u8]>::len);
                                store.cache_bytes += entry.bytes;
                                entry.state = MediaState::Ready(image);
                            }
                            Err(error) => entry.state = MediaState::Failed(error),
                        }
                    }
                    store.trim_cache();
                    store.pump(cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn trim_cache(&mut self) {
        while self.cache_bytes > MAX_CACHE_BYTES
            || self
                .entries
                .values()
                .filter(|entry| matches!(entry.state, MediaState::Ready(_)))
                .count()
                > MAX_READY_ENTRIES
        {
            if !self.evict_oldest() {
                break;
            }
        }
    }

    fn evict_oldest(&mut self) -> bool {
        let oldest = self
            .entries
            .iter()
            .filter(|(_, entry)| !matches!(entry.state, MediaState::Loading))
            .min_by_key(|(_, entry)| entry.touched)
            .map(|(url, _)| url.clone());
        if let Some(url) = oldest {
            let entry = self.entries.remove(&url).unwrap();
            self.cache_bytes -= entry.bytes;
            true
        } else {
            false
        }
    }
}

pub(crate) fn validate_url(value: &str) -> Result<Url, String> {
    let url = Url::parse(value).map_err(|_| "图片地址无效".to_owned())?;
    if url.scheme() != "https"
        || url.host_str() != Some("image.nmb.best")
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !(url.path().starts_with("/thumb/") || url.path().starts_with("/image/"))
        || url.path().ends_with('/')
    {
        return Err("图片地址不属于受支持的图片服务器".to_owned());
    }
    Ok(url)
}

fn image_client() -> Result<&'static Client, String> {
    static CLIENT: OnceLock<Result<Client, String>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            Client::builder()
                .user_agent("nmbxd-rs/0.1")
                .https_only(true)
                .connect_timeout(Duration::from_secs(8))
                .timeout(Duration::from_secs(20))
                .redirect(reqwest::redirect::Policy::none())
                .no_gzip()
                .no_brotli()
                .no_deflate()
                .build()
                .map_err(|_| "无法初始化图片客户端".to_owned())
        })
        .as_ref()
        .map_err(Clone::clone)
}

// All media consumers share this limit, including clipboard and file exports.
struct RequestPermit;
static REQUESTS: (Mutex<usize>, Condvar) = (Mutex::new(0), Condvar::new());

impl RequestPermit {
    fn acquire() -> Self {
        let (count, available) = &REQUESTS;
        let mut active = count.lock().unwrap_or_else(|error| error.into_inner());
        while *active >= MAX_CONCURRENT {
            active = available
                .wait(active)
                .unwrap_or_else(|error| error.into_inner());
        }
        *active += 1;
        Self
    }
}

impl Drop for RequestPermit {
    fn drop(&mut self) {
        let (count, available) = &REQUESTS;
        let mut active = count.lock().unwrap_or_else(|error| error.into_inner());
        *active -= 1;
        available.notify_one();
    }
}

/// Read original, bounded bytes on a worker, without API credentials or redirects.
/// Callers must use `decode_rgba` before trusting them as an image.
pub(crate) fn fetch_bytes(value: &str) -> Result<Vec<u8>, String> {
    let url = validate_url(value)?;
    let _permit = RequestPermit::acquire();
    let response = image_client()?
        .get(url)
        .header(reqwest::header::ACCEPT_ENCODING, "identity")
        .send()
        .map_err(|_| "图片读取失败，请检查网络连接".to_owned())?;
    if !response.status().is_success() {
        return Err(format!("图片服务器返回 HTTP {}", response.status()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY_BYTES as u64)
    {
        return Err("图片文件超过 12 MiB 限制".to_owned());
    }
    read_bounded(response)
}

fn fetch_image(value: &str) -> Result<Arc<RenderImage>, String> {
    decode_image(&fetch_bytes(value)?)
}

fn read_bounded(reader: impl Read) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_BODY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "图片数据读取失败".to_owned())?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err("图片文件超过 12 MiB 限制".to_owned());
    }
    Ok(bytes)
}

fn decode_image(bytes: &[u8]) -> Result<Arc<RenderImage>, String> {
    let (mut pixels, _) = decode_rgba(bytes)?;
    // GPUI's RenderImage expects BGRA pixels.
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    Ok(Arc::new(RenderImage::new(vec![Frame::new(pixels)])))
}

/// Decode one frame with the same format and allocation bounds as the viewer.
/// Callers bound encoded bytes; clipboard PNGs may exceed the network body cap.
pub(crate) fn decode_rgba(bytes: &[u8]) -> Result<(image::RgbaImage, image::ImageFormat), String> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| "无法识别图片格式".to_owned())?;
    if !matches!(
        reader.format(),
        Some(
            image::ImageFormat::Jpeg
                | image::ImageFormat::Png
                | image::ImageFormat::Gif
                | image::ImageFormat::WebP
        )
    ) {
        return Err("暂不支持此图片格式".to_owned());
    }
    let format = reader.format().unwrap();
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_CACHE_BYTES as u64);
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
        || decoder.total_bytes() > MAX_CACHE_BYTES as u64
    {
        return Err("图片尺寸超过限制".to_owned());
    }
    decoder
        .set_limits(limits)
        .map_err(|_| "图片尺寸超过限制".to_owned())?;
    let pixels = DynamicImage::from_decoder(decoder)
        .map_err(|_| "图片解码失败".to_owned())?
        .into_rgba8();
    Ok((pixels, format))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricts_images_to_fixed_https_origin_and_paths() {
        for valid in [
            "https://image.nmb.best/thumb/2026/abc.jpg",
            "https://image.nmb.best/image/abc.png",
        ] {
            assert!(validate_url(valid).is_ok(), "{valid}");
        }
        for invalid in [
            "http://image.nmb.best/thumb/a.jpg",
            "https://image.nmb.best.evil.test/thumb/a.jpg",
            "https://example.com/image/a.jpg",
            "https://image.nmb.best:444/image/a.jpg",
            "https://cookie@image.nmb.best/image/a.jpg",
            "https://image.nmb.best/image/a.jpg?q=1",
            "https://image.nmb.best/image/a.jpg#fragment",
            "https://image.nmb.best/other/a.jpg",
            "https://image.nmb.best/image/../other/a.jpg",
            "https://image.nmb.best/image/",
        ] {
            assert!(validate_url(invalid).is_err(), "{invalid}");
        }
    }

    #[test]
    fn body_limit_also_applies_without_content_length() {
        assert_eq!(read_bounded(Cursor::new(b"image")).unwrap(), b"image");
        assert!(read_bounded(std::io::repeat(0).take(MAX_BODY_BYTES as u64 + 1)).is_err());
    }

    #[test]
    fn decodes_first_frame_and_swaps_red_and_blue() {
        let pixels = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 8, 0, 255]));
        let mut bytes = Cursor::new(Vec::new());
        pixels
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        let decoded = decode_image(bytes.get_ref()).unwrap();
        assert_eq!(decoded.frame_count(), 1);
        assert_eq!(decoded.as_bytes(0).unwrap(), &[0, 8, 255, 255]);
    }

    #[test]
    fn rejects_decoded_dimensions_before_pixel_allocation() {
        let pixels = image::RgbaImage::new(MAX_DIMENSION + 1, 1);
        let mut bytes = Cursor::new(Vec::new());
        pixels
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        assert!(decode_image(bytes.get_ref()).is_err());
    }

    #[test]
    fn cache_eviction_preserves_pending_entries() {
        let mut store = MediaStore::new();
        store.entries.insert(
            "pending".into(),
            Entry {
                state: MediaState::Loading,
                bytes: 0,
                touched: 0,
            },
        );
        store.entries.insert(
            "old-failure".into(),
            Entry {
                state: MediaState::Failed("failure".into()),
                bytes: 0,
                touched: 1,
            },
        );
        assert!(store.evict_oldest());
        assert!(matches!(store.peek("pending"), Some(MediaState::Loading)));
        assert!(store.peek("old-failure").is_none());
        assert!(!store.evict_oldest());
    }
}
