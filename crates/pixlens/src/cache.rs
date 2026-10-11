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

/// Pixels a cell is given in the PNG: enough for a HiDPI screen.
const CELL_WIDTH_PX: u32 = 20;
const CELL_HEIGHT_PX: u32 = 40;

const KEEP_FOR: Duration = Duration::from_secs(14 * 24 * 60 * 60);

/// The folder resized images go in: `PIXLENS_CACHE_DIR` when set, else the
/// platform's cache folder.
pub fn default_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("PIXLENS_CACHE_DIR") {
        return PathBuf::from(dir);
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let base = if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Caches"))
    } else if cfg!(windows) {
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
    } else {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or(home.map(|h| h.join(".cache")))
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
        assert!(converted.starts_with(dir.join("cache")));
        assert_eq!(image::image_dimensions(&converted).unwrap(), (5, 3));

        fs::remove_dir_all(&dir).unwrap();
    }
}
