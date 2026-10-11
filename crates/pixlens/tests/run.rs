//! `cli::run` in process: arguments, terminal size, platform and
//! environment handed in, output captured.

mod common;

use pixlens::cli::{self, Io};
use pixlens::platform::{Env, Platform};
use serde_json::Value;

use common::Scratch;

/// What one run printed and answered.
struct Run {
    code: u8,
    out: String,
    err: String,
}

impl Run {
    fn json(&self) -> Value {
        serde_json::from_str(&self.out).unwrap_or_else(|_| panic!("not JSON: {}", self.out))
    }
}

fn run_on(args: &[&str], terminal: Option<(u32, u32)>, platform: Platform, env: &dyn Env) -> Run {
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let mut io = Io {
        out: &mut out,
        err: &mut err,
        terminal,
        platform,
        env,
    };
    let args = std::iter::once("pixlens")
        .chain(args.iter().copied())
        .map(Into::into)
        .collect();
    let code = cli::run(args, &mut io);
    Run {
        code,
        out: String::from_utf8_lossy(&out).into_owned(),
        err: String::from_utf8_lossy(&err).into_owned(),
    }
}

fn run(args: &[&str], env: &dyn Env) -> Run {
    run_on(args, None, Platform::MacOs, env)
}

#[test]
fn help_and_version_go_to_stdout_and_bad_arguments_to_stderr() {
    let help = run(&["--help"], &[]);
    assert_eq!(help.code, 0);
    assert!(help.out.contains("Usage: pixlens"));

    let version = run(&["--version"], &[]);
    assert_eq!(version.code, 0);
    assert!(version.out.contains(env!("CARGO_PKG_VERSION")));

    let bad = run(&["render"], &[]);
    assert_eq!(bad.code, 2);
    assert!(bad.err.contains("required"));
    assert!(bad.out.is_empty());
}

#[test]
fn show_fits_the_terminal_or_the_given_box() {
    let scratch = Scratch::new("run-show");
    let file = scratch.png("wide.png", 400, 100);
    let path = file.to_str().unwrap();
    let blocks = ["show", path, "--protocol", "blocks", "--colors", "256"];

    let sized = run_on(&blocks, Some((21, 30)), Platform::MacOs, &[]);
    assert_eq!(sized.code, 0);
    assert_eq!(
        sized.out.lines().count(),
        3,
        "20 columns wide is 3 rows tall"
    );

    let default = run_on(&blocks, None, Platform::MacOs, &[]);
    assert_eq!(default.out.lines().count(), 10, "an 80-column default");

    let boxed = run(
        &[&blocks[..], &["--columns", "8", "--rows", "1"]].concat(),
        &[],
    );
    assert_eq!(boxed.out.lines().count(), 1);
}

#[test]
fn show_detects_the_protocol_and_palette_from_the_environment() {
    let scratch = Scratch::new("run-detect");
    let file = scratch.png("a.png", 16, 16);
    let path = file.to_str().unwrap();

    let ghostty = run(&["show", path], &[("TERM_PROGRAM", "ghostty")]);
    assert!(ghostty.out.starts_with("\x1b_G"));

    let apple = run(
        &["show", path, "-W", "4"],
        &[("TERM_PROGRAM", "Apple_Terminal")],
    );
    assert!(apple.out.contains(";5;"), "256 colors in Apple's Terminal");

    let truecolor = run(
        &[
            "show",
            path,
            "-p",
            "blocks",
            "--colors",
            "truecolor",
            "-W",
            "4",
        ],
        &[],
    );
    assert!(truecolor.out.contains(";2;"));
}

#[test]
fn show_names_each_of_several_files_and_reports_the_bad_ones() {
    let scratch = Scratch::new("run-several");
    let good = scratch.png("good.png", 8, 8);
    let missing = scratch.path("missing.png");
    let out = run(
        &[
            "show",
            good.to_str().unwrap(),
            missing.to_str().unwrap(),
            "-p",
            "blocks",
        ],
        &[],
    );
    assert_eq!(out.code, 1);
    assert!(out.out.contains("good.png") && out.out.contains("missing.png"));
    assert!(out.err.contains("pixlens: cannot open"));
}

#[test]
fn render_answers_png_cells_and_overlay_tags() {
    let scratch = Scratch::new("run-render");
    let file = scratch.png("pic.png", 120, 60);
    let cache = scratch.path("cache");
    let (path, cache_dir) = (file.to_str().unwrap(), cache.to_str().unwrap());
    let args = [
        "render",
        path,
        "--columns",
        "12",
        "--rows",
        "6",
        "--png",
        "--cells",
        "--glyphs",
        "half",
        "--overlay",
        "--cache-dir",
        cache_dir,
    ];
    let answer = run(&args, &[]).json();
    assert_eq!(
        (answer["columns"].as_u64(), answer["rows"].as_u64()),
        (Some(12), Some(3))
    );
    assert!(answer["png"].as_str().unwrap().starts_with(cache_dir));
    assert!(answer["cells"].is_string());
    let tag = answer["tag"].as_str().unwrap();
    assert!(tag.starts_with('\u{28FF}') && tag.chars().count() == 4);
    assert_eq!(std::fs::read_dir(cache.join("overlay")).unwrap().count(), 1);

    // Too narrow for a tag: no overlay entry.
    let narrow = run(
        &[
            "render",
            path,
            "--columns",
            "3",
            "--overlay",
            "--cache-dir",
            cache_dir,
        ],
        &[],
    )
    .json();
    assert!(narrow.get("tag").is_none());
}

#[test]
fn render_finds_the_cache_from_the_environment() {
    let scratch = Scratch::new("run-cache");
    let file = scratch.png("pic.png", 10, 10);
    let home = scratch.path("home");
    let env = [("HOME", home.to_str().unwrap())];
    let answer = run(&["render", file.to_str().unwrap(), "--png"], &env).json();
    // Compared as paths, so Windows' `\` and `/` separators both match.
    let png = std::path::Path::new(answer["png"].as_str().unwrap());
    assert!(
        png.starts_with(home.join("Library/Caches/pixlens")),
        "{png:?}"
    );
}

#[test]
fn render_of_a_missing_file_is_an_error_answer() {
    let out = run(
        &["render", "/nonexistent/x.png", "--cache-dir", "/tmp"],
        &[],
    );
    assert_eq!(out.code, 1);
    assert!(
        out.json()["error"]
            .as_str()
            .unwrap()
            .contains("cannot open")
    );
}

#[cfg(unix)]
#[test]
fn setup_prints_or_runs_the_plugin_install() {
    let scratch = Scratch::new("run-setup");
    let dry = run(&["setup", "--dry-run", "--scope", "local"], &[]);
    assert_eq!(
        dry.out.trim(),
        "claude plugin install pixlens --marketplace z0nicx-d9/PixLens --scope local"
    );

    let claude = scratch.tool("claude", "exit 0");
    let claude = claude.to_str().unwrap();
    let mac = run_on(&["setup", "--claude", claude], None, Platform::MacOs, &[]);
    assert_eq!(mac.code, 0, "{}", mac.err);
    assert!(mac.out.contains("pixlens is set up") && mac.out.contains("Accessibility"));
    assert!(
        scratch
            .calls()
            .contains("claude plugin install pixlens --marketplace z0nicx-d9/PixLens --scope user")
    );

    let linux = run_on(
        &["setup", "--claude", claude, "--scope", "project"],
        None,
        Platform::Linux,
        &[],
    );
    assert!(linux.out.contains("pixlens is set up") && !linux.out.contains("Accessibility"));
    assert!(scratch.calls().contains("--scope project"));
}

#[cfg(unix)]
#[test]
fn setup_explains_why_claude_could_not_run() {
    let scratch = Scratch::new("run-setup-fail");
    let failing = scratch.tool("claude-fails", "exit 3");
    let out = run(&["setup", "--claude", failing.to_str().unwrap()], &[]);
    assert_eq!(out.code, 1);
    assert!(out.err.contains("installing the plugin failed"));

    let missing = run(
        &[
            "setup",
            "--claude",
            scratch.path("no-claude").to_str().unwrap(),
        ],
        &[],
    );
    assert_eq!(missing.code, 1);
    assert!(missing.err.contains("Claude Code not found"));

    let not_executable = scratch.path("claude-text");
    std::fs::write(&not_executable, "plain text").unwrap();
    let blocked = run(
        &["setup", "--claude", not_executable.to_str().unwrap()],
        &[],
    );
    assert_eq!(blocked.code, 1);
    assert!(blocked.err.contains("cannot run"));
}

/// A terminal that refuses every write.
struct Broken;

impl std::io::Write for Broken {
    fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
        Err(std::io::Error::other("the terminal went away"))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Err(std::io::Error::other("the terminal went away"))
    }
}

#[test]
fn show_reports_a_terminal_it_cannot_write_to() {
    let scratch = Scratch::new("run-broken");
    let file = scratch.png("a.png", 4, 4);
    let (mut out, mut err) = (Broken, Vec::new());
    let mut io = Io {
        out: &mut out,
        err: &mut err,
        terminal: None,
        platform: Platform::MacOs,
        env: &[],
    };
    let args = ["pixlens", "show", file.to_str().unwrap(), "-p", "blocks"];
    assert_eq!(cli::run(args.iter().map(Into::into).collect(), &mut io), 1);
    assert!(String::from_utf8_lossy(&err).contains("the terminal went away"));
}

/// Every command that writes to the cache fails cleanly when it can't.
#[test]
fn commands_report_a_cache_they_cannot_write() {
    let scratch = Scratch::new("run-cache-blocked");
    let png = scratch.png("a.png", 64, 64);
    let svg = scratch.path("b.svg");
    std::fs::write(
        &svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"/>"#,
    )
    .unwrap();
    let jpeg = scratch.path("c.jpg");
    image::DynamicImage::ImageRgba8(common::gradient(4, 4))
        .into_rgb8()
        .save(&jpeg)
        .unwrap();
    let blocked = scratch.path("not-a-folder");
    std::fs::write(&blocked, "").unwrap();
    let cache = blocked.join("cache");
    let (png, svg, jpeg, cache) = (
        png.to_str().unwrap(),
        svg.to_str().unwrap(),
        jpeg.to_str().unwrap(),
        cache.to_str().unwrap(),
    );

    let mut commands = vec![
        vec!["render", png, "--png", "--cache-dir", cache],
        vec!["render", png, "--overlay", "--cache-dir", cache],
        vec!["copy", jpeg, "--cache-dir", cache],
    ];
    // An SVG goes through a PNG in the cache, when pixlens can read SVG.
    if cfg!(feature = "svg") {
        commands.extend([
            vec!["render", svg, "--overlay", "--cache-dir", cache],
            vec!["open", svg, "--cache-dir", cache],
            vec!["menu", svg, "--cache-dir", cache],
        ]);
    }
    for args in commands {
        let out = run(&args, &[]);
        assert_eq!(out.code, 1, "{args:?}");
        assert!(
            out.json()["error"]
                .as_str()
                .unwrap()
                .contains("cannot create"),
            "{args:?}: {}",
            out.out
        );
    }
}

#[test]
fn commands_report_a_missing_file() {
    for command in ["copy", "open", "menu"] {
        let out = run(&[command, "/nonexistent/a.png", "--cache-dir", "/tmp"], &[]);
        assert_eq!(out.code, 1);
        assert!(
            out.json()["error"]
                .as_str()
                .unwrap()
                .contains("cannot open")
        );
    }
}
