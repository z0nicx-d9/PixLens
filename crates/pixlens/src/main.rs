use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};

use pixlens::blocks::{self, Glyphs, Palette};
use pixlens::load::Loaded;
use pixlens::protocol::{self, Protocol};
use pixlens::{cache, fit_cells, load, overlay, system};

#[derive(Parser)]
#[command(
    name = "pixlens",
    version,
    about = "Image preview for the terminal and for Claude Code"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Draw images in this terminal.
    Show {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// Widest the picture may be, in cells [default: the terminal's width]
        #[arg(short = 'W', long)]
        columns: Option<u32>,
        /// Tallest the picture may be, in cells [default: the terminal's height]
        #[arg(short = 'H', long)]
        rows: Option<u32>,
        /// How to draw [default: detected from the terminal]
        #[arg(short, long)]
        protocol: Option<Protocol>,
        /// Colors for `blocks` [default: truecolor, 256 in Apple's Terminal]
        #[arg(long)]
        colors: Option<Colors>,
        /// Glyphs for `blocks`: quadrants (sharper) or half-blocks
        #[arg(long, default_value = "quad")]
        glyphs: Glyphs,
    },
    /// Fit an image to a box of cells and print it as JSON (what the Claude
    /// Code mod reads).
    Render {
        file: PathBuf,
        /// Widest the picture may be, in cells
        #[arg(long, default_value_t = 60)]
        columns: u32,
        /// Tallest the picture may be, in cells
        #[arg(long, default_value_t = 20)]
        rows: u32,
        /// Write a PNG sized for the box and print its path as `png`
        #[arg(long)]
        png: bool,
        /// Print block cells for a `Raster` element as `cells`
        #[arg(long)]
        cells: bool,
        /// Glyphs for `--cells`: quadrants (sharper) or half-blocks
        #[arg(long, default_value = "quad")]
        glyphs: Glyphs,
        /// Record the picture for Pixlens Overlay and print the `tag` to
        /// draw in its first cells
        #[arg(long)]
        overlay: bool,
        /// Where `--png` writes [default: the platform cache folder]
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
    /// Put an image on the system clipboard, as a PNG whatever its format.
    Copy {
        file: PathBuf,
        /// Where a converted PNG is written [default: the platform cache folder]
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
    /// Start Pixlens Overlay, which draws real pictures over block previews
    /// in Apple's Terminal (macOS), and say whether it is ready to draw.
    Overlay {
        /// The overlay app (`Pixlens Overlay.app`, shipped in the plugin)
        #[arg(long)]
        app: Option<PathBuf>,
        /// Where the overlay's files are [default: the platform cache folder]
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
    /// Show a menu for an image at the pointer (macOS, with Pixlens Overlay):
    /// open in Preview, Quick Look, copy, show in Finder. Elsewhere it opens
    /// the image in the default viewer.
    Menu {
        file: PathBuf,
        /// The overlay app (`Pixlens Overlay.app`, shipped in the plugin)
        #[arg(long)]
        app: Option<PathBuf>,
        /// Where a converted PNG is written [default: the platform cache folder]
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
    /// Open an image in Preview (macOS) or the desktop's default viewer.
    Open {
        file: PathBuf,
        /// Where a converted PNG is written [default: the platform cache folder]
        #[arg(long)]
        cache_dir: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum Colors {
    Truecolor,
    #[value(name = "256")]
    Ansi256,
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Show {
            files,
            columns,
            rows,
            protocol,
            colors,
            glyphs,
        } => {
            let (term_columns, term_rows) = terminal_size::terminal_size()
                .map(|(w, h)| (u32::from(w.0), u32::from(h.0)))
                .unwrap_or((80, 24));
            let max_columns = columns.unwrap_or(term_columns.saturating_sub(1)).max(1);
            let max_rows = rows.unwrap_or(term_rows.saturating_sub(2)).max(1);
            let protocol = protocol.unwrap_or_else(Protocol::detect);
            let palette = match colors {
                Some(Colors::Truecolor) => Palette::TrueColor,
                Some(Colors::Ansi256) => Palette::Ansi256,
                None => Palette::detect(),
            };

            let mut code = ExitCode::SUCCESS;
            for file in &files {
                if files.len() > 1 {
                    println!("\x1b[2m{}\x1b[0m", file.display());
                }
                if let Err(err) = show(file, max_columns, max_rows, protocol, palette, glyphs) {
                    eprintln!("pixlens: {err:#}");
                    code = ExitCode::FAILURE;
                }
            }
            code
        }
        Command::Render {
            file,
            columns,
            rows,
            png,
            cells,
            glyphs,
            overlay,
            cache_dir,
        } => {
            let cache_dir = cache_dir.unwrap_or_else(cache::default_dir);
            let cells = cells.then_some(glyphs);
            answer(render(
                &file, columns, rows, png, cells, overlay, &cache_dir,
            ))
        }
        Command::Overlay { app, cache_dir } => {
            let cache_dir = cache_dir.unwrap_or_else(cache::default_dir);
            answer(overlay::start(app.as_deref(), &cache_dir).map(|(started, ready)| {
                json!({ "overlay": if started { "started" } else { "running" }, "ready": ready })
            }))
        }
        Command::Copy { file, cache_dir } => {
            let cache_dir = cache_dir.unwrap_or_else(cache::default_dir);
            answer(copy(&file, &cache_dir).map(|png| json!({ "copied": png })))
        }
        Command::Menu {
            file,
            app,
            cache_dir,
        } => {
            let cache_dir = cache_dir.unwrap_or_else(cache::default_dir);
            answer(menu(&file, app.as_deref(), &cache_dir))
        }
        Command::Open { file, cache_dir } => {
            let cache_dir = cache_dir.unwrap_or_else(cache::default_dir);
            answer(open(&file, &cache_dir).map(|opened| json!({ "opened": opened })))
        }
    }
}

/// Prints the JSON answer, or `{ "error" }`, and the matching exit code.
fn answer(result: Result<Value>) -> ExitCode {
    match result {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("{}", json!({ "error": format!("{err:#}") }));
            ExitCode::FAILURE
        }
    }
}

fn copy(file: &Path, cache_dir: &Path) -> Result<PathBuf> {
    let loaded = load(file)?;
    let png = cache::full_png(file, &loaded.image, loaded.format, cache_dir)?;
    system::copy_png(&png)?;
    Ok(png)
}

/// Opens the original file. A format the viewer may not read (SVG, QOI) is
/// opened as a full-size PNG instead.
fn open(file: &Path, cache_dir: &Path) -> Result<PathBuf> {
    let target = viewable(file, &load(file)?, cache_dir)?;
    system::open(&target)?;
    Ok(target)
}

fn menu(file: &Path, app: Option<&Path>, cache_dir: &Path) -> Result<Value> {
    let target = viewable(file, &load(file)?, cache_dir)?;
    let original = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
    if overlay::menu(app, &target, &original)? {
        return Ok(json!({ "menu": original }));
    }
    system::open(&target)?;
    Ok(json!({ "opened": target }))
}

/// A file macOS apps read for `file`: the file itself, or for a format
/// they may not (SVG, QOI) a full-size PNG of it.
fn viewable(file: &Path, loaded: &Loaded, cache_dir: &Path) -> Result<PathBuf> {
    let original =
        std::fs::canonicalize(file).with_context(|| format!("cannot find {}", file.display()))?;
    if matches!(loaded.format, "svg" | "qoi") {
        return cache::full_png(file, &loaded.image, loaded.format, cache_dir);
    }
    Ok(original)
}

fn show(
    file: &Path,
    max_columns: u32,
    max_rows: u32,
    protocol: Protocol,
    palette: Palette,
    glyphs: Glyphs,
) -> Result<()> {
    let loaded = load(file)?;
    let cell_box = fit_cells(
        loaded.image.width(),
        loaded.image.height(),
        max_columns,
        max_rows,
    );
    let bytes = protocol::draw(&loaded.image, cell_box, protocol, palette, glyphs)?;
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(&bytes)?;
    stdout.flush()?;
    Ok(())
}

fn render(
    file: &Path,
    columns: u32,
    rows: u32,
    png: bool,
    cells: Option<Glyphs>,
    overlay: bool,
    cache_dir: &Path,
) -> Result<Value> {
    let loaded = load(file)?;
    let (width, height) = loaded.image.dimensions();
    let cell_box = fit_cells(width, height, columns, rows);

    let mut answer = json!({
        "path": std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf()),
        "format": loaded.format,
        "width": width,
        "height": height,
        "columns": cell_box.columns,
        "rows": cell_box.rows,
    });
    if png {
        answer["png"] = json!(cache::png_for(file, &loaded.image, cell_box, cache_dir)?);
    }
    if let Some(glyphs) = cells {
        let drawn = blocks::cells_with(&loaded.image, cell_box, glyphs);
        answer["cells"] = json!(blocks::raster_base64(&drawn));
    }
    if overlay && cell_box.columns >= overlay::TAG_CELLS {
        let id = cache::key(file, cell_box) as u32;
        let shown = viewable(file, &loaded, cache_dir)?;
        let original = std::fs::canonicalize(file).unwrap_or_else(|_| file.to_path_buf());
        overlay::record(id, &shown, &original, cell_box, cache_dir)?;
        answer["tag"] = json!(overlay::tag(id));
    }
    Ok(answer)
}
