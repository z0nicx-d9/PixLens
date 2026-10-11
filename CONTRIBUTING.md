# Contributing to pixlens

Thanks for helping. Issues and pull requests are welcome.

## Layout

```text
.
├── crates/pixlens/          Rust CLI and library
│   ├── src/
│   │   ├── main.rs          thin entry point: reads the terminal, calls cli::run
│   │   ├── cli.rs           arguments (clap) and dispatch
│   │   ├── commands.rs      what each command does
│   │   ├── platform.rs      the desktop and environment, passed in for tests
│   │   ├── load.rs          decoding, EXIF rotation, SVG
│   │   ├── fit.rs           fitting a picture to terminal cells
│   │   ├── blocks.rs        colored block glyphs
│   │   ├── protocol.rs      kitty and iTerm2 graphics, terminal detection
│   │   ├── cache.rs         resized PNGs on disk
│   │   ├── system.rs        clipboard and image viewer
│   │   └── overlay.rs       talking to Pixlens Overlay
│   └── tests/               end-to-end tests (common/ holds shared helpers)
├── plugin/                  the Claude Code plugin
│   ├── .claude-plugin/      plugin manifest
│   ├── hooks/               TypeScript hooks
│   ├── overlay/             Pixlens Overlay (Swift) and its built app
│   ├── tests/               plugin tests
│   └── types/               the plugin's state contract
├── .claude-plugin/          marketplace manifest
├── docs/                    README images and GIFs
└── examples/                sample images
```

## Develop

```console
$ cargo test                       # the CLI, unit and end-to-end
$ cargo clippy --all-targets -- -D warnings
$ cargo fmt --check
$ claude plugin validate .         # marketplace, manifest and hooks
$ claude plugin test plugin        # the plugin's tests
$ claude --plugin-dir ./plugin     # a session with your working copy
$ plugin/overlay/build.sh          # rebuild Pixlens Overlay after changing its Swift
```

## Tests and coverage

Every Rust function is covered by a test. Check before you open a pull request:

```console
$ cargo llvm-cov --summary-only
```

- Platform-specific code takes a `Platform` and an `Env` (see
  `platform.rs`), so its Windows and Linux branches are tested on any machine.
- Commands that start desktop tools (`copy`, `open`, `menu`, `overlay`) are
  tested in `tests/desktop.rs` against stand-in scripts on `PATH`, so nothing
  real opens. Those tests run on macOS.

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org/) in English:
`feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `perf:`, `style:`, `build:`,
`ci:` or `chore:`, then a short summary in the imperative.

## License

By contributing you agree that your work is dual licensed under MIT and
Apache-2.0, as described in the README.
