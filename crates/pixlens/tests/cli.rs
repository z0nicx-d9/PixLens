//! The binary end to end: arguments in, JSON or drawing out.

mod common;

use std::path::PathBuf;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use common::{Scratch, gradient, json, pixlens};

#[cfg(feature = "svg")]
const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="100">
  <rect width="200" height="100" fill="#3366ff"/>
  <circle cx="100" cy="50" r="40" fill="#ffcc00"/>
</svg>"##;

#[test]
fn render_jpeg_answers_box_png_and_cells() {
    let scratch = Scratch::new("jpeg");
    let file = scratch.path("photo.jpg");
    image::DynamicImage::ImageRgba8(gradient(1600, 900))
        .into_rgb8()
        .save(&file)
        .unwrap();

    let out = pixlens(
        &[
            "render",
            file.to_str().unwrap(),
            "--columns",
            "40",
            "--rows",
            "20",
            "--png",
            "--cells",
        ],
        &scratch.path("cache"),
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let answer = json(&out);

    assert_eq!(answer["format"], "jpeg");
    assert_eq!(
        (answer["width"].as_u64(), answer["height"].as_u64()),
        (Some(1600), Some(900))
    );
    assert_eq!(
        (answer["columns"].as_u64(), answer["rows"].as_u64()),
        (Some(40), Some(12))
    );

    let png = PathBuf::from(answer["png"].as_str().unwrap());
    assert!(png.starts_with(scratch.path("cache")));
    assert_eq!(image::image_dimensions(&png).unwrap(), (800, 450));

    let cells = STANDARD.decode(answer["cells"].as_str().unwrap()).unwrap();
    assert_eq!(cells.len(), 40 * 12 * 12);
}

#[cfg(feature = "svg")]
#[test]
fn render_svg() {
    let scratch = Scratch::new("svg");
    let file = scratch.path("chart.svg");
    std::fs::write(&file, SVG).unwrap();

    let out = pixlens(
        &["render", file.to_str().unwrap(), "--columns", "20"],
        &scratch.path("cache"),
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let answer = json(&out);

    assert_eq!(answer["format"], "svg");
    assert_eq!(
        (answer["columns"].as_u64(), answer["rows"].as_u64()),
        (Some(20), Some(5))
    );
    assert!(answer.get("png").is_none() && answer.get("cells").is_none());
}

#[test]
fn render_a_missing_file_answers_an_error() {
    let scratch = Scratch::new("missing");
    let out = pixlens(
        &["render", scratch.path("nope.png").to_str().unwrap()],
        &scratch.path("cache"),
    );

    assert!(!out.status.success());
    assert!(json(&out)["error"].as_str().unwrap().contains("nope.png"));
}

#[test]
fn render_text_is_not_an_image() {
    let scratch = Scratch::new("text");
    let file = scratch.path("notes.png");
    std::fs::write(&file, "not a picture").unwrap();

    let out = pixlens(&["render", file.to_str().unwrap()], &scratch.path("cache"));
    assert!(!out.status.success());
    assert!(json(&out)["error"].is_string());
}

#[test]
fn show_blocks_draws_half_blocks() {
    let scratch = Scratch::new("show");
    let file = scratch.path("small.png");
    gradient(8, 8).save(&file).unwrap();

    let out = pixlens(
        &[
            "show",
            file.to_str().unwrap(),
            "--protocol",
            "blocks",
            "--colors",
            "truecolor",
            "--glyphs",
            "half",
        ],
        &scratch.path("cache"),
    );
    assert!(out.status.success());
    let text = String::from_utf8(out.stdout).unwrap();
    assert_eq!(text.lines().count(), 4);
    assert_eq!(text.matches('▀').count(), 32);
}

#[test]
fn setup_installs_the_plugin_from_this_repository() {
    let scratch = Scratch::new("setup");
    let out = pixlens(&["setup", "--dry-run"], &scratch.path("cache"));
    assert!(out.status.success());
    let line = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        line.trim(),
        "claude plugin install pixlens --marketplace z0nicx-d9/pixlens --scope user"
    );

    let out = pixlens(
        &["setup", "--dry-run", "--scope", "project"],
        &scratch.path("cache"),
    );
    assert!(
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .ends_with("--scope project")
    );
}
