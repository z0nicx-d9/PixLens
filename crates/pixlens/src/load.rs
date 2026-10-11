use std::path::Path;

use anyhow::{Context, Result, bail};
use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage};

/// A decoded picture and the name of the format it came from.
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
    let orientation = decoder.orientation()?;
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

    let mut pixmap = tiny_skia::Pixmap::new(width, height)
        .with_context(|| format!("SVG {} has no drawable size", path.display()))?;
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
    let image = RgbaImage::from_raw(width, height, pixels).context("SVG pixel buffer size")?;

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
