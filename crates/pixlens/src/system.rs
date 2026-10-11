//! Handing an image to the rest of the desktop: the clipboard and the
//! default image viewer. Each platform's own tool does the work, so the
//! binary links no GUI toolkit.

use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::platform::{Env, Platform, ProcessEnv};

/// Puts the PNG at `png` on the system clipboard as an image, which
/// Claude Code's own ctrl+v (and any app's paste) then reads.
pub fn copy_png(png: &Path) -> Result<()> {
    copy_png_on(Platform::CURRENT, png, &ProcessEnv)
}

/// `copy_png` for a given platform and environment.
pub fn copy_png_on(platform: Platform, png: &Path, env: &dyn Env) -> Result<()> {
    run(copy_command(platform, png, env)?)
}

/// Opens `file` in Preview on macOS, or the desktop's default image viewer.
pub fn open(file: &Path) -> Result<()> {
    run(open_command(Platform::CURRENT, file))
}

/// Runs `command` to the end; an error when it can't start or fails.
pub fn run(mut command: Command) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("cannot run {:?}", command.get_program()))?;
    if !status.success() {
        bail!("{:?} failed ({status})", command.get_program());
    }
    Ok(())
}

/// The clipboard tool for `platform`. The path goes as an argument or
/// variable, never spliced into a script, so no quoting can break it.
pub fn copy_command(platform: Platform, png: &Path, env: &dyn Env) -> Result<Command> {
    match platform {
        Platform::MacOs => {
            let mut command = env.command(OsStr::new("osascript"));
            command.args([
                "-e",
                "on run argv",
                "-e",
                "set the clipboard to (read (POSIX file (item 1 of argv)) as «class PNGf»)",
                "-e",
                "end run",
            ]);
            command.arg(png);
            Ok(command)
        }
        Platform::Windows => {
            let mut command = env.command(OsStr::new("powershell"));
            command.args([
                "-NoProfile",
                "-STA",
                "-Command",
                "Add-Type -AssemblyName System.Windows.Forms, System.Drawing; \
                 [System.Windows.Forms.Clipboard]::SetImage([System.Drawing.Image]::FromFile($env:PIXLENS_PNG))",
            ]);
            command.env("PIXLENS_PNG", png);
            Ok(command)
        }
        // Both tools stay in the background to serve the clipboard after we exit.
        Platform::Linux
            if env.var_os("WAYLAND_DISPLAY").is_some() && env.has_program("wl-copy") =>
        {
            let mut command = env.command(OsStr::new("sh"));
            command.args(["-c", "exec wl-copy --type image/png < \"$1\"", "pixlens"]);
            command.arg(png);
            Ok(command)
        }
        Platform::Linux if env.has_program("xclip") => {
            let mut command = env.command(OsStr::new("xclip"));
            command.args(["-selection", "clipboard", "-t", "image/png", "-i"]);
            command.arg(png);
            Ok(command)
        }
        Platform::Linux => bail!("no clipboard tool found: install wl-clipboard or xclip"),
    }
}

/// The command that opens `file` in `platform`'s image viewer.
pub fn open_command(platform: Platform, file: &Path) -> Command {
    let mut command = match platform {
        Platform::MacOs => {
            let mut command = Command::new("open");
            command.args(["-a", "Preview"]);
            command
        }
        Platform::Windows => {
            let mut command = Command::new("cmd");
            command.args(["/C", "start", ""]);
            command
        }
        Platform::Linux => Command::new("xdg-open"),
    };
    command.arg(file);
    command
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    fn args(command: &Command) -> Vec<&OsStr> {
        command.get_args().collect()
    }

    #[test]
    fn macos_copy_passes_the_path_as_an_argument() {
        let command = copy_command(Platform::MacOs, Path::new("/tmp/a \"b\".png"), &[]).unwrap();
        assert_eq!(command.get_program(), "osascript");
        let all = args(&command);
        let (path, script) = all.split_last().unwrap();
        assert_eq!(path, &"/tmp/a \"b\".png");
        assert!(
            script
                .iter()
                .all(|arg| !arg.to_string_lossy().contains("a \"b\""))
        );
    }

    #[test]
    fn windows_copy_hands_the_path_in_a_variable() {
        let command = copy_command(Platform::Windows, Path::new("C:\\a.png"), &[]).unwrap();
        assert_eq!(command.get_program(), "powershell");
        let path = command
            .get_envs()
            .find(|(key, _)| *key == "PIXLENS_PNG")
            .and_then(|(_, value)| value);
        assert_eq!(path, Some(OsStr::new("C:\\a.png")));
    }

    #[test]
    fn linux_copy_prefers_wl_copy_on_wayland_then_xclip() {
        let dir = std::env::temp_dir().join(format!("pixlens-tools-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("wl-copy"), "").unwrap();
        std::fs::write(dir.join("xclip"), "").unwrap();
        let path = dir.to_str().unwrap();
        let png = Path::new("/tmp/a.png");

        let wayland = [("WAYLAND_DISPLAY", "wayland-0"), ("PATH", path)];
        let command = copy_command(Platform::Linux, png, &wayland).unwrap();
        assert_eq!(command.get_program(), "sh");
        assert_eq!(args(&command).last(), Some(&OsStr::new("/tmp/a.png")));

        let command = copy_command(Platform::Linux, png, &[("PATH", path)]).unwrap();
        assert_eq!(command.get_program(), "xclip");

        let error = copy_command(Platform::Linux, png, &[("PATH", "/nowhere")]).unwrap_err();
        assert!(error.to_string().contains("no clipboard tool"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn open_uses_each_platforms_viewer_and_names_the_file_last() {
        let file = Path::new("/tmp/x.png");
        for (platform, program) in [
            (Platform::MacOs, "open"),
            (Platform::Windows, "cmd"),
            (Platform::Linux, "xdg-open"),
        ] {
            let command = open_command(platform, file);
            assert_eq!(command.get_program(), program);
            assert_eq!(args(&command).last(), Some(&OsStr::new("/tmp/x.png")));
        }
    }

    #[cfg(unix)]
    #[test]
    fn copying_runs_the_platforms_clipboard_tool() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("pixlens-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join("osascript");
        std::fs::write(&tool, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&tool, std::fs::Permissions::from_mode(0o755)).unwrap();
        let path = format!("{}:/bin:/usr/bin", dir.display());
        assert!(
            copy_png_on(
                Platform::MacOs,
                Path::new("/tmp/a.png"),
                &[("PATH", path.as_str())]
            )
            .is_ok()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn copying_without_a_clipboard_tool_fails_before_running_anything() {
        let error = copy_png_on(
            Platform::Linux,
            Path::new("/tmp/a.png"),
            &[("PATH", "/nowhere")],
        )
        .unwrap_err();
        assert!(error.to_string().contains("no clipboard tool"));
    }

    #[cfg(unix)]
    #[test]
    fn run_reports_failures() {
        assert!(run(Command::new("true")).is_ok());
        let failed = run(Command::new("false")).unwrap_err();
        assert!(failed.to_string().contains("failed"));
        let missing = run(Command::new("pixlens-no-such-program")).unwrap_err();
        assert!(missing.to_string().contains("cannot run"));
    }
}
