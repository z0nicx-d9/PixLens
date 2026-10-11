//! Resized PNGs for terminals that draw pixels. The terminal reads them by
//! path, so they live in a per-user cache folder rather than a temp file.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use image::RgbaImage;
use image::imageops::{self, FilterType};

use crate::fit::CellBox;
use crate::platform::{Env, Platform, ProcessEnv};

/// Pixels a cell is given in the PNG: enough for a HiDPI screen.
const CELL_WIDTH_PX: u32 = 20;
const CELL_HEIGHT_PX: u32 = 40;

const KEEP_FOR: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// The folder resized images go in, for this machine.
pub fn default_dir() -> PathBuf {
    default_dir_for(Platform::CURRENT, &ProcessEnv)
}

/// The folder resized images go in: `PIXLENS_CACHE_DIR` when set, else
/// `platform`'s cache folder, else the temp folder.
pub fn default_dir_for(platform: Platform, env: &dyn Env) -> PathBuf {
    if let Some(dir) = env.var_os("PIXLENS_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    let home = env.var_os("HOME").map(PathBuf::from);
    let base = match platform {
        Platform::MacOs => home.map(|h| h.join("Library/Caches")),
        Platform::Windows => env.var_os("LOCALAPPDATA").map(PathBuf::from),
        Platform::Linux => env
            .var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or(home.map(|h| h.join(".cache"))),
    };
    base.unwrap_or_else(std::env::temp_dir).join("pixlens")
}

/// Writes `image` scaled for `cell_box` as a PNG in `dir` and answers its
/// path. The name hashes the source file's identity and the box, so a
/// second call for the same picture reuses the file.
pub fn png_for(source: &Path, image: &RgbaImage, cell_box: CellBox, dir: &Path) -> Result<PathBuf> {
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let path = dir.join(format!("{:016x}.png", key(source, cell_box)));
    if path.is_file() {
        return Ok(path);
    }

    let (width, height) = png_size(image.width(), image.height(), cell_box);
    let scaled = if (width, height) == image.dimensions() {
        image.clone()
    } else {
        imageops::resize(image, width, height, FilterType::CatmullRom)
    };
    write_png(&scaled, &path)?;

    prune(dir);
    Ok(path)
}

/// The picture as a PNG at full size, for the clipboard: the source itself
/// when it already is one, else a copy written in `dir`.
pub fn full_png(source: &Path, image: &RgbaImage, format: &str, dir: &Path) -> Result<PathBuf> {
    if format == "png" {
        return Ok(fs::canonicalize(source).unwrap_or_else(|_| source.to_path_buf()));
    }
    fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let full = CellBox {
        columns: 0,
        rows: 0,
    };
    let path = dir.join(format!("full-{:016x}.png", key(source, full)));
    if !path.is_file() {
        write_png(image, &path)?;
    }
    prune(dir);
    Ok(path)
}

/// Writes beside the final name, then renames, so a reader never sees half a file.
fn write_png(image: &RgbaImage, path: &Path) -> Result<()> {
    let partial = path.with_extension(format!("{}.part", std::process::id()));
    image
        .save_with_format(&partial, image::ImageFormat::Png)
        .with_context(|| format!("cannot write {}", partial.display()))?;
    fs::rename(&partial, path).with_context(|| format!("cannot write {}", path.display()))
}

/// The PNG's pixel size: the image shrunk to fit the box at
/// `CELL_*_PX` per cell, never enlarged.
fn png_size(width: u32, height: u32, cell_box: CellBox) -> (u32, u32) {
    let max_w = (cell_box.columns * CELL_WIDTH_PX) as f64;
    let max_h = (cell_box.rows * CELL_HEIGHT_PX) as f64;
    let scale = (max_w / width.max(1) as f64)
        .min(max_h / height.max(1) as f64)
        .min(1.0);
    let scaled = |v: u32| ((v as f64 * scale).round() as u32).max(1);
    (scaled(width), scaled(height))
}

/// Names a source file at one size; changes when the file does.
pub fn key(source: &Path, cell_box: CellBox) -> u64 {
    let mut hasher = DefaultHasher::new();
    fs::canonicalize(source)
        .unwrap_or_else(|_| source.to_path_buf())
        .hash(&mut hasher);
    if let Ok(meta) = fs::metadata(source) {
        meta.len().hash(&mut hasher);
        meta.modified().ok().hash(&mut hasher);
    }
    (cell_box.columns, cell_box.rows).hash(&mut hasher);
    hasher.finish()
}

/// Drops cached PNGs not written for `KEEP_FOR`. Best effort.
fn prune(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let is_old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .is_ok_and(|modified| now.duration_since(modified).unwrap_or_default() > KEEP_FOR);
        if is_old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dir_follows_each_platform() {
        let set = [("PIXLENS_CACHE_DIR", "/c")];
        assert_eq!(default_dir_for(Platform::MacOs, &set), PathBuf::from("/c"));
        let home = [("HOME", "/h")];
        assert_eq!(
            default_dir_for(Platform::MacOs, &home),
            PathBuf::from("/h/Library/Caches/pixlens")
        );
        let local = [("LOCALAPPDATA", "C:/L")];
        assert_eq!(
            default_dir_for(Platform::Windows, &local),
            PathBuf::from("C:/L/pixlens")
        );
        let xdg = [("XDG_CACHE_HOME", "/x"), ("HOME", "/h")];
        assert_eq!(
            default_dir_for(Platform::Linux, &xdg),
            PathBuf::from("/x/pixlens")
        );
        assert_eq!(
            default_dir_for(Platform::Linux, &home),
            PathBuf::from("/h/.cache/pixlens")
        );
        assert_eq!(
            default_dir_for(Platform::Linux, &[]),
            std::env::temp_dir().join("pixlens")
        );
        let _ = default_dir();
    }

    #[test]
    fn prune_drops_files_older_than_two_weeks() {
        let dir = std::env::temp_dir().join(format!("pixlens-prune-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let (old, new) = (dir.join("old.png"), dir.join("new.png"));
        fs::write(&old, "").unwrap();
        fs::write(&new, "").unwrap();
        let weeks_ago = SystemTime::now() - Duration::from_secs(15 * 24 * 60 * 60);
        fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(weeks_ago)
            .unwrap();
        prune(&dir);
        assert!(!old.exists() && new.exists());
        prune(&dir.join("missing"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn writing_into_a_missing_folder_fails() {
        let image = RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        assert!(write_png(&image, Path::new("/nonexistent/dir/a.png")).is_err());
        let blocked = std::env::temp_dir().join(format!("pixlens-file-{}", std::process::id()));
        fs::write(&blocked, "").unwrap();
        let cell_box = CellBox {
            columns: 1,
            rows: 1,
        };
        assert!(png_for(&blocked, &image, cell_box, &blocked.join("cache")).is_err());
        assert!(full_png(&blocked, &image, "jpeg", &blocked.join("cache")).is_err());
        fs::remove_file(blocked).unwrap();
    }

    #[test]
    fn a_missing_source_is_named_as_given() {
        let missing = Path::new("/nonexistent/a.png");
        let image = RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        assert_eq!(
            full_png(missing, &image, "png", Path::new("/tmp")).unwrap(),
            missing
        );
        let cell_box = CellBox {
            columns: 1,
            rows: 1,
        };
        assert_eq!(key(missing, cell_box), key(missing, cell_box));
    }

    #[cfg(unix)]
    #[test]
    fn a_read_only_cache_is_an_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("pixlens-locked-{}", std::process::id()));
        let cache = dir.join("cache");
        fs::create_dir_all(&cache).unwrap();
        let source = dir.join("a.jpg");
        fs::write(&source, "").unwrap();
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o555)).unwrap();
        let image = RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255]));
        let cell_box = CellBox {
            columns: 1,
            rows: 1,
        };
        assert!(png_for(&source, &image, cell_box, &cache).is_err());
        assert!(full_png(&source, &image, "jpeg", &cache).is_err());
        fs::set_permissions(&cache, fs::Permissions::from_mode(0o755)).unwrap();
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_png_cannot_replace_a_folder() {
        let dir = std::env::temp_dir().join(format!("pixlens-rename-{}", std::process::id()));
        let target = dir.join("taken.png");
        fs::create_dir_all(target.join("inside")).unwrap();
        let image = RgbaImage::from_pixel(1, 1, image::Rgba([0, 0, 0, 255]));
        let error = write_png(&image, &target).unwrap_err();
        assert!(format!("{error:#}").contains("cannot write"));
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn png_shrinks_to_the_box_and_keeps_aspect() {
        let cell_box = CellBox {
            columns: 60,
            rows: 17,
        };
        assert_eq!(png_size(4000, 2250, cell_box), (1200, 675));
    }

    #[test]
    fn png_is_not_enlarged() {
        assert_eq!(
            png_size(
                64,
                32,
                CellBox {
                    columns: 64,
                    rows: 16
                }
            ),
            (64, 32)
        );
    }

    #[test]
    fn same_source_and_box_reuse_one_file() {
        let dir = std::env::temp_dir().join(format!("pixlens-test-{}", std::process::id()));
        let source = dir.join("source.png");
        fs::create_dir_all(&dir).unwrap();
        let image = RgbaImage::from_pixel(8, 8, image::Rgba([1, 2, 3, 255]));
        image.save(&source).unwrap();

        let cell_box = CellBox {
            columns: 8,
            rows: 4,
        };
        let first = png_for(&source, &image, cell_box, &dir.join("cache")).unwrap();
        let second = png_for(&source, &image, cell_box, &dir.join("cache")).unwrap();
        assert_eq!(first, second);
        assert!(first.is_file());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn full_png_keeps_a_png_and_converts_the_rest() {
        let dir = std::env::temp_dir().join(format!("pixlens-full-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let image = RgbaImage::from_pixel(5, 3, image::Rgba([9, 8, 7, 255]));
        let png = dir.join("a.png");
        image.save(&png).unwrap();
        let jpeg = dir.join("b.jpg");
        image::DynamicImage::ImageRgba8(image.clone())
            .into_rgb8()
            .save(&jpeg)
            .unwrap();

        assert_eq!(
            full_png(&png, &image, "png", &dir.join("cache")).unwrap(),
            fs::canonicalize(&png).unwrap()
        );
        let converted = full_png(&jpeg, &image, "jpeg", &dir.join("cache")).unwrap();
        assert_eq!(
            full_png(&jpeg, &image, "jpeg", &dir.join("cache")).unwrap(),
            converted
        );
        assert!(converted.starts_with(dir.join("cache")));
        assert_eq!(image::image_dimensions(&converted).unwrap(), (5, 3));

        fs::remove_dir_all(&dir).unwrap();
    }
}
