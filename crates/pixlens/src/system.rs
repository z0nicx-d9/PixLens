//! Handing an image to the rest of the desktop: the clipboard and the
//! default image viewer. Each platform's own tool does the work, so the
//! binary links no GUI toolkit.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Puts the PNG at `png` on the system clipboard as an image, which
/// Claude Code's own ctrl+v (and any app's paste) then reads.
pub fn copy_png(png: &Path) -> Result<()> {
    let mut command = copy_command(png)?;
    let status = command
        .status()
        .with_context(|| format!("cannot run {:?}", command.get_program()))?;
    if !status.success() {
        bail!("{:?} failed ({status})", command.get_program());
    }
    Ok(())
}

/// Opens `file` in Preview on macOS, or the desktop's default image viewer.
pub fn open(file: &Path) -> Result<()> {
    let mut command = open_command(file);
    let status = command
        .status()
        .with_context(|| format!("cannot run {:?}", command.get_program()))?;
    if !status.success() {
        bail!("{:?} failed ({status})", command.get_program());
    }
    Ok(())
}

/// The clipboard tool for this platform. The path goes as an argument or
/// variable, never spliced into a script, so no quoting can break it.
fn copy_command(png: &Path) -> Result<Command> {
    if cfg!(target_os = "macos") {
        let mut command = Command::new("osascript");
        command.args([
            "-e",
            "on run argv",
            "-e",
            "set the clipboard to (read (POSIX file (item 1 of argv)) as «class PNGf»)",
            "-e",
            "end run",
        ]);
        command.arg(png);
        return Ok(command);
    }
    if cfg!(windows) {
        let mut command = Command::new("powershell");
        command.args([
            "-NoProfile",
            "-STA",
            "-Command",
            "Add-Type -AssemblyName System.Windows.Forms, System.Drawing; \
             [System.Windows.Forms.Clipboard]::SetImage([System.Drawing.Image]::FromFile($env:PIXLENS_PNG))",
        ]);
        command.env("PIXLENS_PNG", png);
        return Ok(command);
    }

    // Both tools stay in the background to serve the clipboard after we exit.
    let is_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    if is_wayland && has_program("wl-copy") {
        let mut command = Command::new("sh");
        command.args(["-c", "exec wl-copy --type image/png < \"$1\"", "pixlens"]);
        command.arg(png);
        return Ok(command);
    }
    if has_program("xclip") {
        let mut command = Command::new("xclip");
        command.args(["-selection", "clipboard", "-t", "image/png", "-i"]);
        command.arg(png);
        return Ok(command);
    }
    bail!("no clipboard tool found: install wl-clipboard or xclip")
}

fn open_command(file: &Path) -> Command {
    if cfg!(target_os = "macos") {
        let mut command = Command::new("open");
        command.args(["-a", "Preview"]).arg(file);
        command
    } else if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", ""]);
        command.arg(file);
        command
    } else {
        let mut command = Command::new("xdg-open");
        command.arg(file);
        command
    }
}

fn has_program(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_copy_passes_the_path_as_an_argument() {
        let command = copy_command(Path::new("/tmp/a \"b\".png")).unwrap();
        assert_eq!(command.get_program(), "osascript");
        let args: Vec<_> = command.get_args().collect();
        let (path, script) = args.split_last().unwrap();
        assert_eq!(path, &"/tmp/a \"b\".png");
        assert!(
            script
                .iter()
                .all(|arg| !arg.to_string_lossy().contains("a \"b\""))
        );
    }

    #[test]
    fn open_names_the_file_last() {
        let command = open_command(Path::new("/tmp/x.png"));
        assert_eq!(command.get_args().last().unwrap(), "/tmp/x.png");
    }
}
