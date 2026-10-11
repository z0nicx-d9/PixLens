//! Real pictures over block previews in terminals that cannot draw pixels
//! (Apple's Terminal), drawn by the Pixlens Overlay app (`overlay/`).
//!
//! A block picture carries a tag in its top-left cells: U+28FF, then three
//! braille cells holding a 24-bit id. The app reads the terminal's text
//! through the Accessibility API, finds each tag, looks the id up in the
//! overlay folder, and lays the file named there over the picture's box.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::json;

use crate::fit::CellBox;
use crate::platform::{Env, Platform, ProcessEnv};

/// How many cells a tag takes; narrower pictures get none.
pub const TAG_CELLS: u32 = 4;

/// The overlay app's executable name, inside `Pixlens Overlay.app`.
const PROGRAM: &str = "pixlens-overlay";

/// How many times, 25 ms apart, `start` looks for the app to say it is ready.
const READY_POLLS: u32 = 100;

/// The tag drawn in a picture's first cells for `id` (its low 24 bits).
pub fn tag(id: u32) -> String {
    let mut out = String::from('\u{28FF}');
    for shift in [16, 8, 0] {
        out.push(char::from_u32(0x2800 + ((id >> shift) & 0xFF)).unwrap_or('\u{2800}'));
    }
    out
}

/// Records that tag `id` stands for `viewable` (a file apps read, made from
/// `original`) in a `cell_box`, for the app.
pub fn record(
    id: u32,
    viewable: &Path,
    original: &Path,
    cell_box: CellBox,
    cache_dir: &Path,
) -> Result<()> {
    let dir = cache_dir.join("overlay");
    fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let entry = json!({
        "path": viewable,
        "original": original,
        "columns": cell_box.columns,
        "rows": cell_box.rows,
    });
    let path = dir
        .join(format!("{:06x}", id & 0xFF_FFFF))
        .with_extension("json");
    let partial = path.with_extension(format!("json.{}", std::process::id()));
    fs::write(&partial, entry.to_string())
        .with_context(|| format!("cannot write {}", partial.display()))?;
    fs::rename(&partial, &path).with_context(|| format!("cannot write {}", path.display()))
}

/// Starts the overlay app unless it already runs, and says whether it was
/// started and whether it is ready: allowed to read the screen, so it will
/// draw. A picture is left blank under the overlay only when it is ready.
///
/// It is started as a child, not through Launch Services, so macOS asks the
/// terminal running Claude Code for Accessibility access, not a new app.
pub fn start(app: Option<&Path>, cache_dir: &Path) -> Result<(bool, bool)> {
    start_on(Platform::CURRENT, app, cache_dir, &ProcessEnv)
}

/// `start` for a given platform and environment.
pub fn start_on(
    platform: Platform,
    app: Option<&Path>,
    cache_dir: &Path,
    env: &dyn Env,
) -> Result<(bool, bool)> {
    if platform != Platform::MacOs {
        bail!("the overlay is for macOS only");
    }
    if is_running() {
        if is_ready(cache_dir) {
            return Ok((false, true));
        }
        // Started from a terminal without Accessibility access, it can never
        // draw; started again from this one, it may.
        let _ = Command::new("pkill").args(["-x", PROGRAM]).status();
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let program = find(platform, app, env).context("Pixlens Overlay.app not found")?;
    let mut command = Command::new(program);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command.spawn().context("cannot start the overlay app")?;
    // It says it is ready within a few frames of starting; a first start,
    // which macOS checks before running, can take a second or two.
    for _ in 0..READY_POLLS {
        if is_ready(cache_dir) {
            return Ok((true, true));
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    Ok((true, false))
}

/// Shows the overlay app's menu for a picture at the pointer: open in
/// Preview, Quick Look, copy, show in Finder. False when the app is not
/// installed, so the caller can fall back.
pub fn menu(app: Option<&Path>, viewable: &Path, original: &Path) -> Result<bool> {
    menu_on(Platform::CURRENT, app, viewable, original, &ProcessEnv)
}

/// `menu` for a given platform and environment.
pub fn menu_on(
    platform: Platform,
    app: Option<&Path>,
    viewable: &Path,
    original: &Path,
    env: &dyn Env,
) -> Result<bool> {
    let Some(program) = find(platform, app, env) else {
        return Ok(false);
    };
    let mut command = Command::new(program);
    command
        .arg("--menu")
        .arg(viewable)
        .arg(original)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command.spawn().context("cannot start the overlay app")?;
    Ok(true)
}

/// Whether the overlay app wrote that it may read the screen, and that
/// process still runs.
fn is_ready(cache_dir: &Path) -> bool {
    let Ok(text) = fs::read_to_string(cache_dir.join("overlay/ready")) else {
        return false;
    };
    text.trim().parse::<u32>().is_ok_and(|pid| {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    })
}

fn is_running() -> bool {
    Command::new("pgrep")
        .args(["-x", PROGRAM])
        .stdout(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// The app's executable, from `app` (the `.app` the Claude Code plugin ships,
/// or the executable itself), else `PIXLENS_OVERLAY`; none off macOS.
fn find(platform: Platform, app: Option<&Path>, env: &dyn Env) -> Option<PathBuf> {
    if platform != Platform::MacOs {
        return None;
    }
    let given = app
        .map(Path::to_path_buf)
        .or_else(|| env.var_os("PIXLENS_OVERLAY").map(PathBuf::from))?;
    let program = if given.extension().is_some_and(|ext| ext == "app") {
        given.join("Contents/MacOS").join(PROGRAM)
    } else {
        given
    };
    program.is_file().then_some(program)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tag_is_a_start_cell_and_three_braille_bytes() {
        assert_eq!(tag(0x0a0b0c), "\u{28FF}\u{280A}\u{280B}\u{280C}");
        assert_eq!(tag(0xff_0a0b0c).chars().count(), TAG_CELLS as usize);
    }

    #[test]
    fn find_takes_an_app_bundle_or_its_executable_on_macos_only() {
        let dir = std::env::temp_dir().join(format!("pixlens-find-{}", std::process::id()));
        let inside = dir.join("Pixlens Overlay.app/Contents/MacOS");
        fs::create_dir_all(&inside).unwrap();
        fs::write(inside.join(PROGRAM), "").unwrap();
        let app = dir.join("Pixlens Overlay.app");
        let program = inside.join(PROGRAM);

        assert_eq!(
            find(Platform::MacOs, Some(&app), &[]),
            Some(program.clone())
        );
        assert_eq!(
            find(Platform::MacOs, Some(&program), &[]),
            Some(program.clone())
        );
        let by_env = [("PIXLENS_OVERLAY", program.to_str().unwrap())];
        assert_eq!(find(Platform::MacOs, None, &by_env), Some(program.clone()));
        assert_eq!(find(Platform::MacOs, None, &[]), None);
        assert_eq!(
            find(Platform::MacOs, Some(&dir.join("missing.app")), &[]),
            None
        );
        assert_eq!(find(Platform::Linux, Some(&app), &[]), None);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn off_macos_there_is_no_overlay() {
        let dir = Path::new("/nonexistent");
        let error = start_on(Platform::Windows, None, dir, &[]).unwrap_err();
        assert!(error.to_string().contains("macOS only"));
        let shown = menu_on(Platform::Linux, None, dir, dir, &[]).unwrap();
        assert!(!shown);
    }

    #[test]
    fn record_reports_where_it_could_not_write() {
        let dir = std::env::temp_dir().join(format!("pixlens-record-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let cell_box = CellBox {
            columns: 4,
            rows: 1,
        };
        let file = Path::new("/x/a.png");

        // The cache folder is a file.
        let blocked = dir.join("file");
        fs::write(&blocked, "").unwrap();
        let error = record(1, file, file, cell_box, &blocked).unwrap_err();
        assert!(format!("{error:#}").contains("cannot create"));

        // The entry's name is taken by a folder.
        fs::create_dir_all(dir.join("taken/overlay/000002.json/inside")).unwrap();
        let error = record(2, file, file, cell_box, &dir.join("taken")).unwrap_err();
        assert!(format!("{error:#}").contains("cannot write"));

        // The overlay folder can't be written to.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let locked = dir.join("locked/overlay");
            fs::create_dir_all(&locked).unwrap();
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o555)).unwrap();
            let error = record(3, file, file, cell_box, &dir.join("locked")).unwrap_err();
            assert!(format!("{error:#}").contains("cannot write"));
            fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        }

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn record_writes_what_the_app_reads() {
        let dir = std::env::temp_dir().join(format!("pixlens-overlay-{}", std::process::id()));
        let cell_box = CellBox {
            columns: 12,
            rows: 3,
        };
        record(
            0x12_3456,
            Path::new("/x/a.png"),
            Path::new("/x/a.svg"),
            cell_box,
            &dir,
        )
        .unwrap();
        let text = fs::read_to_string(dir.join("overlay/123456.json")).unwrap();
        let entry: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            entry,
            json!({ "path": "/x/a.png", "original": "/x/a.svg", "columns": 12, "rows": 3 })
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
