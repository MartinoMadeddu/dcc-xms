//! Texture files for the viewport.
//!
//! Decoding a set of large images takes seconds, so it happens on
//! background threads. The viewport asks for a file; until it is ready the
//! surface shows its plain colour. `generation` moves each time a file
//! finishes, which is the viewport's cue to rebuild its materials.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Largest side kept. Larger images are reduced: the viewport is a preview.
pub const MAX_SIDE: u32 = 2048;

/// A decoded image with its chain of half-size copies, ready for the GPU.
#[derive(Debug)]
pub struct Decoded {
    pub width:  u32,
    pub height: u32,
    /// How many sizes `pixels` holds, the full size first.
    pub levels: u32,
    /// RGBA, eight bits a channel, every level one after the other.
    pub pixels: Vec<u8>,
    /// True when some pixel is not fully opaque.
    pub has_alpha: bool,
}

enum State { Loading, Ready(Arc<Decoded>), Failed }

static CACHE: Mutex<Option<HashMap<PathBuf, State>>> = Mutex::new(None);
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Counts finished files and changes of display mode.
pub fn generation() -> u64 { GENERATION.load(Ordering::Relaxed) }

/// Have the viewport rebuild what it shows.
pub fn touch() { GENERATION.fetch_add(1, Ordering::Relaxed); }

/// Files still being decoded.
pub fn pending() -> usize {
    CACHE.lock().ok().and_then(|c| c.as_ref().map(|m| m.values().filter(|s| matches!(s, State::Loading)).count())).unwrap_or(0)
}

/// Half-size copy of an RGBA image, each pixel the mean of the four it covers.
fn halve(src: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; (nw * nh * 4) as usize];
    for y in 0..nh {
        for x in 0..nw {
            let (x0, y0) = ((x * 2).min(w - 1), (y * 2).min(h - 1));
            let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
            for c in 0..4 {
                let at = |px: u32, py: u32| src[((py * w + px) * 4 + c) as usize] as u32;
                out[((y * nw + x) * 4 + c) as usize] = ((at(x0, y0) + at(x1, y0) + at(x0, y1) + at(x1, y1) + 2) / 4) as u8;
            }
        }
    }
    (out, nw, nh)
}

/// An RGBA image with its smaller copies down to one pixel.
pub fn with_levels(pixels: Vec<u8>, width: u32, height: u32) -> Decoded {
    let has_alpha = pixels.chunks_exact(4).any(|p| p[3] < 250);
    let mut all = pixels;
    let (mut start, mut w, mut h, mut levels) = (0usize, width, height, 1u32);
    while w > 1 || h > 1 {
        let (next, nw, nh) = halve(&all[start..], w, h);
        start = all.len();
        all.extend_from_slice(&next);
        (w, h) = (nw, nh);
        levels += 1;
    }
    Decoded { width, height, levels, pixels: all, has_alpha }
}

fn decode(path: &Path) -> Option<Decoded> {
    let bytes = std::fs::read(path).ok()?;
    let mut img = image::load_from_memory(&bytes).ok()?;
    if img.width().max(img.height()) > MAX_SIDE {
        img = img.resize(MAX_SIDE, MAX_SIDE, image::imageops::FilterType::Triangle);
    }
    let rgba = img.into_rgba8();
    let (w, h) = rgba.dimensions();
    Some(with_levels(rgba.into_raw(), w, h))
}

/// The decoded file, if it is ready. The first call for a file starts its
/// decoding and returns nothing.
pub fn request(path: &Path) -> Option<Arc<Decoded>> {
    let mut guard = CACHE.lock().ok()?;
    let cache = guard.get_or_insert_with(HashMap::new);
    match cache.get(path) {
        Some(State::Ready(d)) => return Some(d.clone()),
        Some(_) => return None,
        None => {}
    }
    cache.insert(path.to_path_buf(), State::Loading);
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let state = match decode(&path) { Some(d) => State::Ready(Arc::new(d)), None => State::Failed };
        if let Ok(mut guard) = CACHE.lock() { guard.get_or_insert_with(HashMap::new).insert(path, state); }
        GENERATION.fetch_add(1, Ordering::Relaxed);
    });
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_run_down_to_one_pixel() {
        // A 4 x 2 image: left half white, right half black, opaque.
        let mut px = vec![];
        for _ in 0..2 { for x in 0..4 { let v = if x < 2 { 255 } else { 0 }; px.extend([v, v, v, 255]); } }
        let d = with_levels(px, 4, 2);
        assert_eq!(d.levels, 3);
        assert_eq!(d.pixels.len(), (4 * 2 + 2 * 1 + 1) * 4);
        assert!(!d.has_alpha);
        // The second level is a white pixel and a black one; the last their mean.
        assert_eq!(&d.pixels[32..40], &[255, 255, 255, 255, 0, 0, 0, 255]);
        assert_eq!(d.pixels[40], 128);
    }

    #[test]
    fn a_file_is_decoded_in_the_background_and_then_kept() {
        let dir = std::env::temp_dir().join("xms_texture_test");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("dot.png");
        let img = image::RgbaImage::from_fn(8, 8, |x, _| image::Rgba([x as u8 * 30, 0, 0, if x == 0 { 0 } else { 255 }]));
        img.save(&file).unwrap();
        let before = generation();
        assert!(request(&file).is_none(), "the first call only starts the work");
        let mut got = None;
        for _ in 0..200 { got = request(&file); if got.is_some() { break; } std::thread::sleep(std::time::Duration::from_millis(10)); }
        let d = got.expect("decoded");
        assert_eq!((d.width, d.height, d.levels), (8, 8, 4));
        assert!(d.has_alpha && generation() > before);
        // A file that is not an image fails quietly and is not retried forever.
        let bad = dir.join("bad.png");
        std::fs::write(&bad, b"not an image").unwrap();
        for _ in 0..100 { if request(&bad).is_none() && pending() == 0 { break; } std::thread::sleep(std::time::Duration::from_millis(10)); }
        assert!(request(&bad).is_none());
    }
}
