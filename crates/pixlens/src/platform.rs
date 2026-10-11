//! Which desktop pixlens runs on, and the environment it reads.
//!
//! Code that differs between macOS, Windows and Linux takes a `Platform` and
//! an `Env` instead of asking the build target and the process, so every
//! branch can be exercised on any machine.

use std::ffi::OsString;
use std::path::PathBuf;

/// A desktop family: what opens files, holds the clipboard and keeps caches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    MacOs,
    Windows,
    /// Linux and the other Unix desktops (freedesktop tools).
    Linux,
}

impl Platform {
    /// The platform this binary was built for.
    #[cfg(target_os = "macos")]
    pub const CURRENT: Platform = Platform::MacOs;
    /// The platform this binary was built for.
    #[cfg(windows)]
    pub const CURRENT: Platform = Platform::Windows;
    /// The platform this binary was built for.
    #[cfg(not(any(target_os = "macos", windows)))]
    pub const CURRENT: Platform = Platform::Linux;
}

/// Environment variables, read through a function so tests can hand in
/// their own.
pub trait Env {
    fn var_os(&self, name: &str) -> Option<OsString>;

    /// The variable as text; empty when unset or not Unicode.
    fn var(&self, name: &str) -> String {
        self.var_os(name)
            .and_then(|value| value.into_string().ok())
            .unwrap_or_default()
    }

    /// Whether an executable named `name` is in one of `PATH`'s folders.
    fn has_program(&self, name: &str) -> bool {
        self.var_os("PATH").is_some_and(|paths| {
            std::env::split_paths(&paths).any(|dir: PathBuf| dir.join(name).is_file())
        })
    }
}

/// The real process environment.
#[derive(Debug, Clone, Copy)]
pub struct ProcessEnv;

impl Env for ProcessEnv {
    fn var_os(&self, name: &str) -> Option<OsString> {
        std::env::var_os(name)
    }
}

/// Fixed variables, for tests and for callers that want to pin some.
impl<const N: usize> Env for [(&str, &str); N] {
    fn var_os(&self, name: &str) -> Option<OsString> {
        self.iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| OsString::from(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_variables_answer_by_name() {
        let env = [("TERM", "xterm-kitty"), ("EMPTY", "")];
        assert_eq!(env.var("TERM"), "xterm-kitty");
        assert_eq!(env.var("EMPTY"), "");
        assert_eq!(env.var("MISSING"), "");
        assert_eq!(env.var_os("MISSING"), None);
    }

    #[test]
    fn has_program_looks_in_path() {
        let dir = std::env::temp_dir().join(format!("pixlens-path-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("tool"), "").unwrap();
        let path = dir.to_str().unwrap();
        assert!([("PATH", path)].has_program("tool"));
        assert!(![("PATH", path)].has_program("other"));
        assert!(![("HOME", "/")].has_program("tool"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_process_env_is_the_real_one() {
        assert_eq!(ProcessEnv.var_os("PATH"), std::env::var_os("PATH"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn current_is_macos_on_macos() {
        assert_eq!(Platform::CURRENT, Platform::MacOs);
    }

    #[cfg(windows)]
    #[test]
    fn current_is_windows_on_windows() {
        assert_eq!(Platform::CURRENT, Platform::Windows);
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    #[test]
    fn current_is_linux_elsewhere() {
        assert_eq!(Platform::CURRENT, Platform::Linux);
    }
}
