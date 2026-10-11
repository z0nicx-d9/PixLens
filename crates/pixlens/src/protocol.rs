//! How `pixlens show` puts pixels on a terminal.

use std::io::Cursor;

use anyhow::Result;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use image::RgbaImage;
use image::imageops::{self, FilterType};

use crate::blocks::{self, Glyphs, Palette};
use crate::fit::CellBox;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Protocol {
    /// The kitty graphics protocol (kitty, Ghostty, Konsole).
    Kitty,
    /// iTerm2 inline images (iTerm2, WezTerm, VS Code with images on).
    Iterm,
    /// Colored block characters: any terminal.
    Blocks,
}

impl Protocol {
    /// Picks the protocol from the environment. Inside tmux or screen the
    /// graphics escapes do not pass through, so those get blocks.
    pub fn detect() -> Self {
        let var = |name: &str| std::env::var(name).unwrap_or_default();
        if !var("TMUX").is_empty() || var("TERM").starts_with("screen") {
            return Protocol::Blocks;
        }
        let program = var("TERM_PROGRAM");
        if !var("KITTY_WINDOW_ID").is_empty()
            || var("TERM") == "xterm-kitty"
            || program == "ghostty"
        {
            Protocol::Kitty
        } else if program == "iTerm.app" || program == "WezTerm" || var("LC_TERMINAL") == "iTerm2" {
            Protocol::Iterm
        } else {
            Protocol::Blocks
        }
    }
}

/// The bytes that draw `image` over `cell_box`, ending on a new line.
pub fn draw(
    image: &RgbaImage,
    cell_box: CellBox,
    protocol: Protocol,
    palette: Palette,
    glyphs: Glyphs,
) -> Result<Vec<u8>> {
    Ok(match protocol {
        Protocol::Blocks => blocks::ansi(
            &blocks::cells_with(image, cell_box, glyphs),
            cell_box.columns,
            palette,
        )
        .into_bytes(),
        Protocol::Kitty => kitty(&png(image, cell_box)?, cell_box),
        Protocol::Iterm => iterm(&png(image, cell_box)?, cell_box),
    })
}

/// A PNG small enough to send inline: at most 24 x 48 pixels a cell.
fn png(image: &RgbaImage, cell_box: CellBox) -> Result<Vec<u8>> {
    let (max_w, max_h) = (cell_box.columns * 24, cell_box.rows * 48);
    let image = if image.width() > max_w || image.height() > max_h {
        let scale = (max_w as f64 / image.width() as f64).min(max_h as f64 / image.height() as f64);
        let w = ((image.width() as f64 * scale).round() as u32).max(1);
        let h = ((image.height() as f64 * scale).round() as u32).max(1);
        imageops::resize(image, w, h, FilterType::CatmullRom)
    } else {
        image.clone()
    };
    let mut out = Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png)?;
    Ok(out.into_inner())
}

/// Kitty: `a=T` transmits and places at once, `c`/`r` size it in cells,
/// and the payload goes in chunks of at most 4096 base64 bytes.
fn kitty(png: &[u8], cell_box: CellBox) -> Vec<u8> {
    let encoded = STANDARD.encode(png);
    let chunks: Vec<&[u8]> = encoded.as_bytes().chunks(4096).collect();
    let mut out = Vec::with_capacity(encoded.len() + chunks.len() * 32);
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i + 1 < chunks.len());
        let header = if i == 0 {
            format!(
                "\x1b_Gf=100,a=T,q=2,c={},r={},m={more};",
                cell_box.columns, cell_box.rows
            )
        } else {
            format!("\x1b_Gm={more};")
        };
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
    out.push(b'\n');
    out
}

/// iTerm2: OSC 1337 with the whole file, sized in cells.
fn iterm(png: &[u8], cell_box: CellBox) -> Vec<u8> {
    let header = format!(
        "\x1b]1337;File=inline=1;size={};width={};height={};preserveAspectRatio=1:",
        png.len(),
        cell_box.columns,
        cell_box.rows
    );
    let mut out = header.into_bytes();
    out.extend_from_slice(STANDARD.encode(png).as_bytes());
    out.extend_from_slice(b"\x07\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kitty_splits_payload_into_chunks() {
        let out = String::from_utf8(kitty(
            &[0u8; 6000],
            CellBox {
                columns: 10,
                rows: 5,
            },
        ))
        .unwrap();
        assert!(out.starts_with("\x1b_Gf=100,a=T,q=2,c=10,r=5,m=1;"));
        assert_eq!(out.matches("\x1b_G").count(), 2);
        assert!(out.contains("\x1b_Gm=0;"));
        assert!(out.ends_with("\x1b\\\n"));
    }

    #[test]
    fn iterm_names_size_and_cells() {
        let out = String::from_utf8(iterm(
            &[1, 2, 3],
            CellBox {
                columns: 4,
                rows: 2,
            },
        ))
        .unwrap();
        assert_eq!(
            out,
            "\x1b]1337;File=inline=1;size=3;width=4;height=2;preserveAspectRatio=1:AQID\x07\n"
        );
    }
}
