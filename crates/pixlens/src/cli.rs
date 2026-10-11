//! The command line: arguments, and which command they run.

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::blocks::{Glyphs, Palette};
use crate::commands;
use crate::platform::{Env, Platform};
use crate::protocol::Protocol;

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
    /// Code plugin reads).
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
    /// Open an image in Preview (macOS) or the desktop's default viewer.
    Open {
        file: PathBuf,
        /// Where a converted PNG is written [default: the platform cache folder]
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
    /// Install the pixlens plugin into Claude Code, from this project's
    /// marketplace, so `cargo install pixlens` is all you run by hand.
    Setup {
        /// Who gets the plugin: you everywhere (user), this project for
        /// everyone (project), or this project for you (local)
        #[arg(long, value_enum, default_value_t = Scope::User)]
        scope: Scope,
        /// The Claude Code executable, if `claude` is not on PATH
        #[arg(long, default_value = "claude")]
        claude: PathBuf,
        /// Print the command instead of running it
        #[arg(long)]
        dry_run: bool,
    },
}

/// Who gets the plugin `setup` installs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Scope {
    User,
    Project,
    Local,
}

#[derive(Clone, Copy, ValueEnum)]
enum Colors {
    Truecolor,
    #[value(name = "256")]
    Ansi256,
}

/// Where a run writes, and what it knows of its surroundings.
#[allow(
    missing_debug_implementations,
    reason = "holds writers, which have no Debug"
)]
pub struct Io<'a> {
    pub out: &'a mut dyn Write,
    pub err: &'a mut dyn Write,
    /// The terminal's size in cells, when stdout is one.
    pub terminal: Option<(u32, u32)>,
    pub platform: Platform,
    pub env: &'a dyn Env,
}

/// Parses `args` (the program name first) and runs the command; answers
/// the process exit code.
pub fn run(args: Vec<OsString>, io: &mut Io) -> u8 {
    let cli = match Cli::try_parse_from(args) {
        Ok(cli) => cli,
        Err(error) => {
            // Help and --version are not errors: they go to stdout.
            let text = error.render().to_string();
            let sink = if error.use_stderr() {
                &mut io.err
            } else {
                &mut io.out
            };
            let _ = write!(sink, "{text}");
            return u8::try_from(error.exit_code()).unwrap_or(1);
        }
    };
    let (platform, env) = (io.platform, io.env);
    let cache =
        |dir: Option<PathBuf>| dir.unwrap_or_else(|| crate::cache::default_dir_for(platform, env));

    match cli.command {
        Command::Show {
            files,
            columns,
            rows,
            protocol,
            colors,
            glyphs,
        } => {
            let palette = match colors {
                Some(Colors::Truecolor) => Palette::TrueColor,
                Some(Colors::Ansi256) => Palette::Ansi256,
                None => Palette::detect_in(env),
            };
            let protocol = protocol.unwrap_or_else(|| Protocol::detect_in(env));
            let options = commands::ShowOptions {
                columns,
                rows,
                protocol,
                palette,
                glyphs,
            };
            commands::show(&files, &options, io)
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
            let options = commands::RenderOptions {
                columns,
                rows,
                png,
                cells: cells.then_some(glyphs),
                overlay,
            };
            let dir = cache(cache_dir);
            commands::answer(commands::render(&file, &options, &dir), io)
        }
        Command::Copy { file, cache_dir } => {
            let dir = cache(cache_dir);
            commands::answer(commands::copy(&file, &dir), io)
        }
        Command::Open { file, cache_dir } => {
            let dir = cache(cache_dir);
            commands::answer(commands::open(&file, &dir), io)
        }
        Command::Menu {
            file,
            app,
            cache_dir,
        } => {
            let dir = cache(cache_dir);
            commands::answer(commands::menu(&file, app.as_deref(), &dir), io)
        }
        Command::Overlay { app, cache_dir } => {
            let dir = cache(cache_dir);
            commands::answer(commands::overlay(app.as_deref(), &dir), io)
        }
        Command::Setup {
            scope,
            claude,
            dry_run,
        } => commands::setup(scope, &claude, dry_run, io),
    }
}
