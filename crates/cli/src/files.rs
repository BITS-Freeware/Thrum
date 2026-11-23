//! Local file handling for attachments.
//!
//! Thrum is chat-first: images are resized and recompressed before they
//! are encrypted and uploaded, so large photos do not waste quota or
//! bandwidth. Received files are saved under sanitized names.

use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::imageops::FilterType;

/// Longest image side kept as-is; anything longer is scaled down.
pub const MAX_IMAGE_SIDE: u32 = 2048;
/// Images larger than this are recompressed even when already small enough.
pub const RECOMPRESS_ABOVE_BYTES: usize = 1024 * 1024;
/// JPEG quality used when recompressing.
pub const JPEG_QUALITY: u8 = 85;

/// A file ready to encrypt and upload.
pub struct PreparedFile {
    pub bytes: Vec<u8>,
    pub name: String,
    pub mime: String,
}

/// Read a file and prepare it for upload. Images above the size or
/// dimension thresholds are scaled to fit [`MAX_IMAGE_SIDE`] and
/// recompressed as JPEG; everything else passes through untouched.
pub fn prepare_upload(path: &Path) -> Result<PreparedFile, String> {
    let bytes =
        std::fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let raw_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let name = sanitize_name(raw_name);
    match recompress_image(&bytes) {
        Some(resized) => Ok(PreparedFile {
            name: with_jpeg_extension(&name),
            mime: "image/jpeg".to_string(),
            bytes: resized,
        }),
        None => {
            let mime = mime_from_name(&name);
            Ok(PreparedFile { name, mime, bytes })
        }
    }
}

/// Recompress an image when it is worth it: `Some(jpeg_bytes)` when the
/// input decodes as an image and exceeds a threshold, `None` when the file
/// should travel as-is. Encoding failures also pass the original through.
fn recompress_image(bytes: &[u8]) -> Option<Vec<u8>> {
    if bytes.len() <= RECOMPRESS_ABOVE_BYTES {
        // A small file needs no work, whatever it is.
        return None;
    }
    let image = image::load_from_memory(bytes).ok()?;
    let (width, height) = (image.width(), image.height());
    let longest = width.max(height);
    let resized = if longest > MAX_IMAGE_SIDE {
        let scale = MAX_IMAGE_SIDE as f32 / longest as f32;
        let new_width = ((width as f32 * scale).round() as u32).max(1);
        let new_height = ((height as f32 * scale).round() as u32).max(1);
        image.resize_exact(new_width, new_height, FilterType::Triangle)
    } else {
        image
    };
    let mut out = Cursor::new(Vec::new());
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, JPEG_QUALITY);
    encoder
        .encode_image(&resized)
        .ok()
        .map(|_| out.into_inner())
}

/// Save received bytes under `dir` with a collision-free variant of `name`,
/// readable by the owner only, in a directory only the owner can open.
/// Returns the path that was written.
pub fn save_attachment(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    private_dir(dir)?;
    let name = sanitize_name(name);
    let (stem, extension) = split_name(&name);
    let mut counter = 1;
    let mut candidate = dir.join(&name);
    loop {
        match create_private(&candidate) {
            Ok(mut file) => {
                use std::io::Write;
                file.write_all(bytes)
                    .and_then(|()| file.sync_all())
                    .map_err(|error| format!("cannot write {}: {error}", candidate.display()))?;
                return Ok(candidate);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                counter += 1;
                candidate = dir.join(match extension.as_deref() {
                    Some(extension) => format!("{stem}-{counter}.{extension}"),
                    None => format!("{stem}-{counter}"),
                });
            }
            Err(error) => {
                return Err(format!("cannot write {}: {error}", candidate.display()));
            }
        }
    }
}

/// Create `dir` (and its parents) if needed; the directory itself is
/// made owner-only.
pub fn private_dir(dir: &Path) -> Result<(), String> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(dir)
        .map_err(|error| format!("cannot create {dir:?}: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("cannot protect {dir:?}: {error}"))?;
    }
    Ok(())
}

/// Create a new file that only the owner can read, failing if it exists.
fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Strip anything that could escape the attachments directory: only the
/// final path component survives, capped to a sane length.
pub fn sanitize_name(raw: &str) -> String {
    let component = Path::new(raw)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let trimmed = component.trim();
    let name = if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        "file"
    } else {
        trimmed
    };
    let mut bounded: String = name.chars().take(255).collect();
    if bounded.is_empty() {
        bounded = "file".to_string();
    }
    bounded.to_string()
}

/// A coarse MIME guess from the file extension.
pub fn mime_from_name(name: &str) -> String {
    let extension = split_name(name).1.unwrap_or_default().to_lowercase();
    match extension.as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "md" => "text/plain",
        "html" => "text/html",
        "mp3" => "audio/mpeg",
        "ogg" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        _ => "application/octet-stream",
    }
    .to_string()
}

fn with_jpeg_extension(name: &str) -> String {
    match split_name(name) {
        (stem, Some(_)) => format!("{stem}.jpg"),
        (stem, None) => format!("{stem}.jpg"),
    }
}

/// Split `name.jpg` into ("name", Some("jpg")).
fn split_name(name: &str) -> (String, Option<String>) {
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() && !extension.is_empty() => {
            (stem.to_string(), Some(extension.to_string()))
        }
        _ => (name.to_string(), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_temp(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn non_image_passes_through() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_temp(dir.path(), "notes.txt", b"plain text attachment");
        let prepared = prepare_upload(&path).unwrap();
        assert_eq!(prepared.name, "notes.txt");
        assert_eq!(prepared.mime, "text/plain");
        assert_eq!(prepared.bytes, b"plain text attachment");
    }

    #[test]
    fn small_images_pass_through() {
        let dir = tempfile::tempdir().unwrap();
        let image = image::DynamicImage::new_rgb8(64, 48);
        let mut png = Vec::new();
        image
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let path = write_temp(dir.path(), "tiny.png", &png);
        let prepared = prepare_upload(&path).unwrap();
        assert_eq!(prepared.name, "tiny.png");
        assert_eq!(prepared.mime, "image/png");
        assert_eq!(prepared.bytes, png);
    }

    #[test]
    fn large_images_are_scaled_and_recompressed() {
        let dir = tempfile::tempdir().unwrap();
        // 3000x2000 noise does not compress as PNG and exceeds both limits.
        let mut noise = image::ImageBuffer::new(3000, 2000);
        let mut state = 0x12345678u32;
        for (_x, _y, pixel) in noise.enumerate_pixels_mut() {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *pixel = image::Rgb([state as u8, (state >> 8) as u8, (state >> 16) as u8]);
        }
        let source = image::DynamicImage::ImageRgb8(noise);
        let mut png = Vec::new();
        source
            .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        assert!(png.len() > RECOMPRESS_ABOVE_BYTES, "png size {}", png.len());
        let path = write_temp(dir.path(), "photo.png", &png);
        let prepared = prepare_upload(&path).unwrap();
        assert_eq!(prepared.name, "photo.jpg");
        assert_eq!(prepared.mime, "image/jpeg");
        assert!(prepared.bytes.len() < png.len(), "recompression shrinks");
        let decoded = image::load_from_memory(&prepared.bytes).unwrap();
        assert!(decoded.width().max(decoded.height()) <= MAX_IMAGE_SIDE);
    }

    #[test]
    fn sanitize_strips_paths_and_dots() {
        assert_eq!(sanitize_name("../../etc/passwd"), "passwd");
        assert_eq!(sanitize_name(".."), "file");
        assert_eq!(sanitize_name("."), "file");
        assert_eq!(sanitize_name("/abs/path/photo.jpg"), "photo.jpg");
        assert_eq!(sanitize_name(""), "file");
    }

    #[test]
    fn save_deduplicates_names() {
        let dir = tempfile::tempdir().unwrap();
        let first = save_attachment(dir.path(), "photo.jpg", b"one").unwrap();
        let second = save_attachment(dir.path(), "photo.jpg", b"two").unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), b"one");
        assert_eq!(std::fs::read(&second).unwrap(), b"two");
        assert!(
            first
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("photo")
        );
    }

    #[test]
    fn mime_guesses() {
        assert_eq!(mime_from_name("a.PNG"), "image/png");
        assert_eq!(mime_from_name("song.mp3"), "audio/mpeg");
        assert_eq!(mime_from_name("blob.bin"), "application/octet-stream");
        assert_eq!(mime_from_name("noext"), "application/octet-stream");
    }
}
