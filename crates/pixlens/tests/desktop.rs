//! The commands that hand images to the desktop, run against stand-in
//! tools on `PATH` so nothing real opens, copies or starts.
#![cfg(target_os = "macos")]

mod common;

use std::path::Path;
use std::process::Output;

use common::{Scratch, json, pixlens_with, text};

/// Runs pixlens with only the stand-in tools (and the shell) on `PATH`.
fn run(scratch: &Scratch, args: &[&str], env: &[(&str, &str)]) -> Output {
    let path = format!("{}:/bin:/usr/bin", scratch.path("bin").display());
    let mut all = vec![("PATH", path.as_str())];
    all.extend_from_slice(env);
    pixlens_with(args, &scratch.path("cache"), &all)
}

fn arg(path: &Path) -> &str {
    path.to_str().unwrap()
}

#[test]
fn copy_hands_a_png_to_osascript() {
    let scratch = Scratch::new("desk-copy");
    scratch.tool("osascript", "exit 0");
    let file = scratch.png("a.png", 4, 4);

    let out = run(&scratch, &["copy", arg(&file)], &[]);
    assert!(out.status.success(), "{}", text(&out.stdout));
    assert!(json(&out)["copied"].as_str().unwrap().ends_with("a.png"));
    assert!(scratch.calls().contains("osascript -e on run argv"));
}

#[test]
fn copy_converts_other_formats_and_reports_a_failing_tool() {
    let scratch = Scratch::new("desk-copy-fail");
    scratch.tool("osascript", "exit 1");
    let file = scratch.path("a.jpg");
    image::DynamicImage::ImageRgba8(common::gradient(4, 4))
        .into_rgb8()
        .save(&file)
        .unwrap();

    let out = run(&scratch, &["copy", arg(&file)], &[]);
    assert!(!out.status.success());
    assert!(json(&out)["error"].as_str().unwrap().contains("failed"));
    let converted = std::fs::read_dir(scratch.path("cache")).unwrap().count();
    assert_eq!(converted, 1, "the JPEG went through a PNG");
}

#[test]
fn open_uses_preview_and_converts_svg_first() {
    let scratch = Scratch::new("desk-open");
    scratch.tool("open", "exit 0");
    let png = scratch.png("a.png", 4, 4);
    let svg = scratch.path("b.svg");
    std::fs::write(
        &svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"/>"#,
    )
    .unwrap();

    let out = run(&scratch, &["open", arg(&png)], &[]);
    assert!(json(&out)["opened"].as_str().unwrap().ends_with("a.png"));
    let out = run(&scratch, &["open", arg(&svg)], &[]);
    assert!(json(&out)["opened"].as_str().unwrap().contains("cache"));
    assert!(scratch.calls().contains("open -a Preview"));

    let missing = run(&scratch, &["open", arg(&scratch.path("none.png"))], &[]);
    assert!(json(&missing)["error"].is_string());
}

#[test]
fn menu_starts_the_overlay_app_or_falls_back_to_opening() {
    let scratch = Scratch::new("desk-menu");
    scratch.tool("open", "exit 0");
    let app = scratch.tool("pixlens-overlay", "exit 0");
    let file = scratch.png("a.png", 4, 4);

    let out = run(&scratch, &["menu", arg(&file), "--app", arg(&app)], &[]);
    assert!(json(&out)["menu"].as_str().unwrap().ends_with("a.png"));

    let without = run(&scratch, &["menu", arg(&file)], &[]);
    assert!(
        json(&without)["opened"]
            .as_str()
            .unwrap()
            .ends_with("a.png")
    );

    // An app that cannot start is an error, not a silent fallback.
    let broken = scratch.path("broken");
    std::fs::write(&broken, "not a program").unwrap();
    let out = run(&scratch, &["menu", arg(&file), "--app", arg(&broken)], &[]);
    assert!(
        json(&out)["error"]
            .as_str()
            .unwrap()
            .contains("cannot start")
    );
}

/// Stand-ins for the tools `overlay` asks: whether the app runs, whether
/// its pid lives, and stopping it.
fn process_tools(scratch: &Scratch, is_running: bool) {
    scratch.tool("pgrep", if is_running { "exit 0" } else { "exit 1" });
    scratch.tool("kill", "exit 0");
    scratch.tool("pkill", "exit 0");
}

/// An overlay app that says it is ready by writing the ready file.
fn ready_app(scratch: &Scratch) -> std::path::PathBuf {
    let ready = scratch.path("cache/overlay/ready");
    let body = format!(
        "mkdir -p '{}'; echo 4242 > '{}'",
        ready.parent().unwrap().display(),
        ready.display()
    );
    scratch.tool("pixlens-overlay", &body)
}

#[test]
fn overlay_starts_the_app_and_waits_until_it_is_ready() {
    let scratch = Scratch::new("desk-overlay");
    process_tools(&scratch, false);
    let app = ready_app(&scratch);
    // macOS checks a new executable on its first run, which can take longer
    // than the wait for readiness: run it once, then clear what it wrote.
    std::process::Command::new(&app).status().unwrap();
    std::fs::remove_file(scratch.path("cache/overlay/ready")).unwrap();

    let out = run(&scratch, &["overlay", "--app", arg(&app)], &[]);
    assert_eq!(
        json(&out),
        serde_json::json!({ "overlay": "started", "ready": true })
    );
}

#[test]
fn overlay_already_running_and_ready_is_left_alone() {
    let scratch = Scratch::new("desk-overlay-running");
    process_tools(&scratch, true);
    let app = ready_app(&scratch);
    std::fs::create_dir_all(scratch.path("cache/overlay")).unwrap();
    std::fs::write(scratch.path("cache/overlay/ready"), "4242").unwrap();

    let out = run(&scratch, &["overlay", "--app", arg(&app)], &[]);
    assert_eq!(
        json(&out),
        serde_json::json!({ "overlay": "running", "ready": true })
    );
    assert!(!scratch.calls().contains("pkill"));
}

#[test]
fn overlay_that_cannot_draw_is_restarted_and_may_stay_unready() {
    let scratch = Scratch::new("desk-overlay-stuck");
    process_tools(&scratch, true);
    // Never writes the ready file, and an old one names no number.
    let app = scratch.tool("pixlens-overlay", "exit 0");
    std::fs::create_dir_all(scratch.path("cache/overlay")).unwrap();
    std::fs::write(scratch.path("cache/overlay/ready"), "not a pid").unwrap();

    let out = run(&scratch, &["overlay", "--app", arg(&app)], &[]);
    assert_eq!(
        json(&out),
        serde_json::json!({ "overlay": "started", "ready": false })
    );
    assert!(scratch.calls().contains("pkill -x pixlens-overlay"));
}

#[test]
fn overlay_without_its_app_is_an_error() {
    let scratch = Scratch::new("desk-overlay-missing");
    process_tools(&scratch, false);
    let out = run(
        &scratch,
        &["overlay", "--app", arg(&scratch.path("none.app"))],
        &[],
    );
    assert!(json(&out)["error"].as_str().unwrap().contains("not found"));

    let broken = scratch.path("broken");
    std::fs::write(&broken, "not a program").unwrap();
    let out = run(&scratch, &["overlay", "--app", arg(&broken)], &[]);
    assert!(
        json(&out)["error"]
            .as_str()
            .unwrap()
            .contains("cannot start")
    );
}

#[test]
fn show_in_a_real_terminal_fits_its_size() {
    let scratch = Scratch::new("desk-tty");
    let file = scratch.png("a.png", 400, 100);
    // `script` gives the binary a terminal of its own, 41 columns wide, so
    // the picture is drawn 40 cells across.
    let command = format!(
        "stty cols 41 rows 30; exec '{}' show '{}' -p blocks --glyphs half",
        env!("CARGO_BIN_EXE_pixlens"),
        arg(&file)
    );
    let out = std::process::Command::new("script")
        .args(["-q", "/dev/null", "sh", "-c", &command])
        .env("PIXLENS_CACHE_DIR", scratch.path("cache"))
        .output()
        .unwrap();
    assert!(out.status.success());
    let first = text(&out.stdout)
        .lines()
        .find(|line| line.contains('▀'))
        .unwrap()
        .to_string();
    assert_eq!(first.matches('▀').count(), 40);
}
