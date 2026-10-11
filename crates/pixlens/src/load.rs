use std::path::Path;

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage};

/// A decoded picture and the name of the format it came from.
#[derive(Debug)]
pub struct Loaded {
    pub image: RgbaImage,
    pub format: &'static str,
}

/// File extensions `load` reads; the mod uses the same list to spot images.
pub const EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "ico", "qoi", "svg", "svgz",
];

/// Decodes the file at `path` to RGBA, turned upright by its EXIF orientation.
///
/// Raster formats are sniffed from their bytes, so a misnamed file still
/// loads. SVG is drawn with resvg when the `svg` feature is on.
pub fn load(path: &Path) -> Result<Loaded> {
    if is_svg(path) {
        return load_svg(path);
    }

    let reader = ImageReader::open(path)
        .with_context(|| format!("cannot open {}", path.display()))?
        .with_guessed_format()
        .with_context(|| format!("cannot read {}", path.display()))?;
    let format = match reader.format() {
        Some(format) => format_name(format),
        None => bail!("{} is not an image pixlens can read", path.display()),
    };

    let mut decoder = reader
        .into_decoder()
        .with_context(|| format!("cannot decode {}", path.display()))?;
    // EXIF that can't be read leaves the picture as stored.
    let orientation = decoder
        .orientation()
        .unwrap_or(image::metadata::Orientation::NoTransforms);
    let mut image = DynamicImage::from_decoder(decoder)
        .with_context(|| format!("cannot decode {}", path.display()))?;
    image.apply_orientation(orientation);

    Ok(Loaded {
        image: image.into_rgba8(),
        format,
    })
}

fn is_svg(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg") || ext.eq_ignore_ascii_case("svgz"))
}

fn format_name(format: image::ImageFormat) -> &'static str {
    use image::ImageFormat::*;
    match format {
        Png => "png",
        Jpeg => "jpeg",
        Gif => "gif",
        WebP => "webp",
        Bmp => "bmp",
        Tiff => "tiff",
        Ico => "ico",
        Qoi => "qoi",
        _ => "image",
    }
}

#[cfg(feature = "svg")]
fn load_svg(path: &Path) -> Result<Loaded> {
    use std::sync::Arc;

    use resvg::{tiny_skia, usvg};

    let data = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;

    // Loading the system fonts takes about a second, so only for text.
    // A compressed .svgz cannot be searched and always gets them.
    let mut fonts = usvg::fontdb::Database::new();
    let is_compressed = data.starts_with(&[0x1f, 0x8b]);
    if is_compressed || data.windows(5).any(|window| window == b"<text") {
        fonts.load_system_fonts();
    }
    let options = usvg::Options {
        resources_dir: path.parent().map(Path::to_path_buf),
        fontdb: Arc::new(fonts),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_data(&data, &options)
        .with_context(|| format!("cannot parse SVG {}", path.display()))?;

    // Vectors scale for free: draw small ones up to 1024 px, cap big ones at 2048.
    let size = tree.size();
    let longest = size.width().max(size.height());
    let scale = if longest < 1024.0 {
        1024.0 / longest
    } else {
        (2048.0 / longest).min(1.0)
    };
    let width = (size.width() * scale).round().max(1.0) as u32;
    let height = (size.height() * scale).round().max(1.0) as u32;

    // Both sides are 1 to 2048 pixels, which always fits.
    let mut pixmap = tiny_skia::Pixmap::new(width, height).expect("a 1..=2048 px pixmap");
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    let pixels = pixmap
        .pixels()
        .iter()
        .flat_map(|pixel| {
            let color = pixel.demultiply();
            [color.red(), color.green(), color.blue(), color.alpha()]
        })
        .collect();
    let image =
        RgbaImage::from_raw(width, height, pixels).expect("one RGBA pixel per pixmap pixel");

    Ok(Loaded {
        image,
        format: "svg",
    })
}

#[cfg(not(feature = "svg"))]
fn load_svg(path: &Path) -> Result<Loaded> {
    bail!(
        "{}: this pixlens was built without SVG support",
        path.display()
    )
}

#[cfg(test)]
mod tests {
    use image::{ImageFormat, Rgba};

    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("pixlens-load-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn every_raster_format_is_named() {
        let dir = scratch("formats");
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(4, 4, Rgba([10, 20, 30, 255])));
        for (format, name, file) in [
            (ImageFormat::Png, "png", "a.png"),
            (ImageFormat::Jpeg, "jpeg", "a.jpg"),
            (ImageFormat::Gif, "gif", "a.gif"),
            (ImageFormat::WebP, "webp", "a.webp"),
            (ImageFormat::Bmp, "bmp", "a.bmp"),
            (ImageFormat::Tiff, "tiff", "a.tif"),
            (ImageFormat::Ico, "ico", "a.ico"),
            (ImageFormat::Qoi, "qoi", "a.qoi"),
        ] {
            let path = dir.join(file);
            let pixels = if format == ImageFormat::Jpeg {
                DynamicImage::ImageRgb8(image.to_rgb8())
            } else {
                image.clone()
            };
            pixels.save_with_format(&path, format).unwrap();
            let loaded = load(&path).unwrap();
            assert_eq!(
                (loaded.format, loaded.image.dimensions()),
                (name, (4, 4)),
                "{file}"
            );
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_format_pixlens_was_not_built_for_is_named_but_not_decoded() {
        let dir = scratch("pnm");
        let path = dir.join("a.ppm");
        std::fs::write(&path, b"P6\n1 1\n255\n\x00\x00\x00").unwrap();
        let error = load(&path).err().unwrap();
        assert!(format!("{error:#}").contains("cannot decode"));
        assert_eq!(format_name(ImageFormat::Pnm), "image");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unreadable_and_broken_files_say_what_went_wrong() {
        let dir = scratch("broken");
        // A folder opens on Unix, but can't be read.
        #[cfg(unix)]
        assert!(format!("{:#}", load(&dir).err().unwrap()).contains("cannot read"));
        // A PNG header with nothing after it.
        let cut = dir.join("cut.png");
        let mut bytes = Vec::new();
        DynamicImage::ImageRgba8(RgbaImage::new(8, 8))
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        std::fs::write(&cut, &bytes[..40]).unwrap();
        assert!(format!("{:#}", load(&cut).err().unwrap()).contains("cannot decode"));
        // Whole chunks, but the pixel data scrambled.
        let scrambled = dir.join("scrambled.png");
        let end = bytes.len() - 16;
        for byte in &mut bytes[end - 8..end] {
            *byte ^= 0xff;
        }
        std::fs::write(&scrambled, &bytes).unwrap();
        assert!(format!("{:#}", load(&scrambled).err().unwrap()).contains("cannot decode"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_missing_file_cannot_be_opened() {
        let error = load(Path::new("/nonexistent/a.png")).err().unwrap();
        assert!(format!("{error:#}").contains("cannot open"));
    }

    #[test]
    fn text_is_not_an_image() {
        let dir = scratch("text");
        let path = dir.join("notes.txt");
        std::fs::write(&path, "hello").unwrap();
        assert!(format!("{:#}", load(&path).err().unwrap()).contains("not an image"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(feature = "svg")]
    #[test]
    fn svg_is_drawn_at_a_useful_size_with_fonts_only_for_text() {
        let dir = scratch("svg");
        let small = dir.join("small.SVG");
        std::fs::write(&small, r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="50"><text y="20">Hi</text></svg>"#).unwrap();
        let loaded = load(&small).unwrap();
        assert_eq!(
            (loaded.format, loaded.image.dimensions()),
            ("svg", (1024, 512))
        );

        let big = dir.join("big.svg");
        std::fs::write(
            &big,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="4096" height="1024"/>"#,
        )
        .unwrap();
        assert_eq!(load(&big).unwrap().image.dimensions(), (2048, 512));

        let broken = dir.join("broken.svgz");
        std::fs::write(&broken, [0x1f, 0x8b, 0, 0]).unwrap();
        assert!(format!("{:#}", load(&broken).err().unwrap()).contains("cannot parse SVG"));

        assert!(load(&dir.join("missing.svg")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
