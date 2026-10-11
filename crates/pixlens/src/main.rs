use std::process::ExitCode;

use pixlens::cli::{self, Io};
use pixlens::platform::{Platform, ProcessEnv};

fn main() -> ExitCode {
    let terminal = terminal_size::terminal_size()
        .map(|(width, height)| (u32::from(width.0), u32::from(height.0)));
    let (mut out, mut err) = (std::io::stdout().lock(), std::io::stderr().lock());
    let mut io = Io {
        out: &mut out,
        err: &mut err,
        terminal,
        platform: Platform::CURRENT,
        env: &ProcessEnv,
    };
    ExitCode::from(cli::run(std::env::args_os().collect(), &mut io))
}
