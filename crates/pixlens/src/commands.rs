//! What each `pixlens` command does. The JSON-speaking commands answer a
//! `Value` for `answer` to print; `show` and `setup` write as they go.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use serde_json::{Value, json};

use crate::blocks::{self, Glyphs, Palette};
use crate::cli::{Io, Scope};
use crate::load::Loaded;
use crate::platform::Platform;
use crate::protocol::{self, Protocol};
use crate::{cache, fit_cells, load, overlay, system};

/// How `show` draws.
#[derive(Debug, Clone, Copy)]
pub struct ShowOptions {
    /// Widest and tallest, in cells; the terminal's size when unset.
    pub columns: Option<u32>,
    pub rows: Option<u32>,
    pub protocol: Protocol,
    pub palette: Palette,
    pub glyphs: Glyphs,
}

/// What `render` answers besides the size.
#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub columns: u32,
    pub rows: u32,
    /// Write a PNG sized for the box.
    pub png: bool,
    /// Block cells in these glyphs.
    pub cells: Option<Glyphs>,
    /// Record the picture for Pixlens Overlay and answer its tag.
    pub overlay: bool,
}

/// Draws each file to `io.out`, naming each when there are several; 1 when
/// any could not be drawn.
pub fn show(files: &[PathBuf], options: &ShowOptions, io: &mut Io) -> u8 {
    let (term_columns, term_rows) = io.terminal.unwrap_or((80, 24));
    let columns = options
        .columns
        .unwrap_or(term_columns.saturating_sub(1))
        .max(1);
    let rows = options.rows.unwrap_or(term_rows.saturating_sub(2)).max(1);

    let mut code = 0;
    for file in files {
        if files.len() > 1 {
            let _ = writeln!(io.out, "\x1b[2m{}\x1b[0m", file.display());
        }
        let drawn = load(file).and_then(|loaded| {
            let cell_box = fit_cells(loaded.image.width(), loaded.image.height(), columns, rows);
            protocol::draw(
                &loaded.image,
                cell_box,
                options.protocol,
                options.palette,
                options.glyphs,
            )
        });
        match drawn.and_then(|bytes| Ok(io.out.write_all(&bytes)?)) {
            Ok(()) => {}
            Err(error) => {
                let _ = writeln!(io.err, "pixlens: {error:#}");
                code = 1;
            }
        }
    }
    let _ = io.out.flush();
    code
}

/// Fits `file` to the box and answers its size and what `options` ask for.
pub fn render(file: &Path, options: &RenderOptions, cache_dir: &Path) -> Result<Value> {
    let loaded = load(file)?;
    let (width, height) = loaded.image.dimensions();
    let cell_box = fit_cells(width, height, options.columns, options.rows);

    let mut answer = json!({
        "path": canonical(file),
        "format": loaded.format,
        "width": width,
        "height": height,
        "columns": cell_box.columns,
        "rows": cell_box.rows,
    });
    if options.png {
        answer["png"] = json!(cache::png_for(file, &loaded.image, cell_box, cache_dir)?);
    }
    if let Some(glyphs) = options.cells {
        let drawn = blocks::cells_with(&loaded.image, cell_box, glyphs);
        answer["cells"] = json!(blocks::raster_base64(&drawn));
    }
    if options.overlay && cell_box.columns >= overlay::TAG_CELLS {
        let id = cache::key(file, cell_box) as u32;
        let shown = viewable(file, &loaded, cache_dir)?;
        overlay::record(id, &shown, &canonical(file), cell_box, cache_dir)?;
        answer["tag"] = json!(overlay::tag(id));
    }
    Ok(answer)
}

/// Puts `file` on the clipboard as a PNG; answers the PNG copied.
pub fn copy(file: &Path, cache_dir: &Path) -> Result<Value> {
    let loaded = load(file)?;
    let png = cache::full_png(file, &loaded.image, loaded.format, cache_dir)?;
    system::copy_png(&png)?;
    Ok(json!({ "copied": png }))
}

/// Opens the original file. A format the viewer may not read (SVG, QOI) is
/// opened as a full-size PNG instead.
pub fn open(file: &Path, cache_dir: &Path) -> Result<Value> {
    let target = viewable(file, &load(file)?, cache_dir)?;
    system::open(&target)?;
    Ok(json!({ "opened": target }))
}

/// Shows Pixlens Overlay's menu for `file`, or opens it where there is none.
pub fn menu(file: &Path, app: Option<&Path>, cache_dir: &Path) -> Result<Value> {
    let target = viewable(file, &load(file)?, cache_dir)?;
    let original = canonical(file);
    if overlay::menu(app, &target, &original)? {
        return Ok(json!({ "menu": original }));
    }
    system::open(&target)?;
    Ok(json!({ "opened": target }))
}

/// Starts Pixlens Overlay; answers whether it was started and is ready.
pub fn overlay(app: Option<&Path>, cache_dir: &Path) -> Result<Value> {
    let (started, ready) = overlay::start(app, cache_dir)?;
    Ok(json!({ "overlay": if started { "started" } else { "running" }, "ready": ready }))
}

/// Runs `claude plugin install pixlens --marketplace <this repo>`, letting
/// Claude Code ask what it asks, then says what is left to do by hand.
pub fn setup(scope: Scope, claude: &Path, dry_run: bool, io: &mut Io) -> u8 {
    let scope = match scope {
        Scope::User => "user",
        Scope::Project => "project",
        Scope::Local => "local",
    };
    let args = [
        "plugin",
        "install",
        "pixlens",
        "--marketplace",
        marketplace(),
        "--scope",
        scope,
    ];
    if dry_run {
        let _ = writeln!(io.out, "{} {}", claude.display(), args.join(" "));
        return 0;
    }

    let status = match Command::new(claude).args(args).status() {
        Ok(status) => status,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let _ = writeln!(
                io.err,
                "pixlens: Claude Code not found ({}): install it from https://claude.com/claude-code, \
                 or pass its path with --claude",
                claude.display()
            );
            return 1;
        }
        Err(error) => {
            let _ = writeln!(io.err, "pixlens: cannot run {}: {error}", claude.display());
            return 1;
        }
    };
    if !status.success() {
        let _ = writeln!(io.err, "pixlens: installing the plugin failed ({status})");
        return 1;
    }

    let _ = writeln!(
        io.out,
        "\npixlens is set up. Start a new Claude Code session to use it."
    );
    if io.platform == Platform::MacOs {
        let _ = writeln!(
            io.out,
            "For sharp pictures in Apple's Terminal, give it (or herdr) Accessibility access:\n\
             System Settings > Privacy & Security > Accessibility."
        );
    }
    0
}

/// Prints the JSON answer, or `{ "error" }`; answers the exit code.
pub fn answer(result: Result<Value>, io: &mut Io) -> u8 {
    match result {
        Ok(value) => {
            let _ = writeln!(io.out, "{value}");
            0
        }
        Err(error) => {
            let _ = writeln!(io.out, "{}", json!({ "error": format!("{error:#}") }));
            1
        }
    }
}

/// Where the plugin's marketplace lives: this crate's own GitHub repository,
/// as the `owner/repo` Claude Code reads (an https URL could be taken for a
/// marketplace.json address).
pub fn marketplace() -> &'static str {
    let repository = env!("CARGO_PKG_REPOSITORY");
    repository
        .strip_prefix("https://github.com/")
        .unwrap_or(repository)
        .trim_end_matches(".git")
}

/// A file macOS apps read for `file`: the file itself, or for a format
/// they may not (SVG, QOI) a full-size PNG of it.
fn viewable(file: &Path, loaded: &Loaded, cache_dir: &Path) -> Result<PathBuf> {
    if matches!(loaded.format, "svg" | "qoi") {
        return cache::full_png(file, &loaded.image, loaded.format, cache_dir);
    }
    Ok(canonical(file))
}

/// `file` as an absolute path when it exists, else as given.
fn canonical(file: &Path) -> PathBuf {
    std::fs::canonicalize(file).unwrap_or(file.to_path_buf())
}
