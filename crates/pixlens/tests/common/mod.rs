//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use image::{Rgba, RgbaImage};
use serde_json::Value;

/// A scratch folder, removed when dropped.
pub struct Scratch(pub PathBuf);

impl Scratch {
    pub fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("pixlens-it-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    pub fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// A PNG of `width` x `height` in the folder.
    pub fn png(&self, name: &str, width: u32, height: u32) -> PathBuf {
        let file = self.path(name);
        gradient(width, height).save(&file).unwrap();
        file
    }

    /// An executable shell script, standing in for a desktop tool: it logs
    /// its name and arguments to `calls.log` and runs `body`.
    pub fn tool(&self, name: &str, body: &str) -> PathBuf {
        let bin = self.path("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let file = bin.join(name);
        let log = self.path("calls.log");
        let script = format!(
            "#!/bin/sh\necho \"{name} $*\" >> '{}'\n{body}\n",
            log.display()
        );
        std::fs::write(&file, script).unwrap();
        executable(&file);
        file
    }

    /// What the stand-in tools were called with, one call a line.
    pub fn calls(&self) -> String {
        std::fs::read_to_string(self.path("calls.log")).unwrap_or_default()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
pub fn executable(file: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
pub fn executable(_file: &Path) {}

pub fn gradient(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        Rgba([(x * 255 / width) as u8, (y * 255 / height) as u8, 128, 255])
    })
}

/// Runs the pixlens binary with its cache in `cache`.
pub fn pixlens(args: &[&str], cache: &Path) -> Output {
    pixlens_with(args, cache, &[])
}

/// Runs the pixlens binary with extra environment variables.
pub fn pixlens_with(args: &[&str], cache: &Path, env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_pixlens"))
        .args(args)
        .env("PIXLENS_CACHE_DIR", cache)
        .envs(env.iter().copied())
        .output()
        .unwrap()
}

pub fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|_| panic!("not JSON: {}", String::from_utf8_lossy(&output.stdout)))
}

pub fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}
