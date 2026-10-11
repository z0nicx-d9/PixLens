//! Character-cell drawing for terminals with no graphics protocol: each
//! cell is a block glyph in two colors. Half-blocks (`▀`) give a cell two
//! pixels stacked; quadrants (`▘▝▖▗` and their unions) give it four, as
//! two by two, so edges come out twice as fine across.

use std::fmt::Write as _;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use image::imageops::{self, FilterType};
use image::{Rgba, RgbaImage};

use crate::fit::CellBox;

const UPPER_HALF: u32 = 0x2580;
const LOWER_HALF: u32 = 0x2584;
const SPACE: u32 = 0x20;

/// A Raster color meaning "the terminal's default" (bit 24 alone).
const DEFAULT_COLOR: u32 = 0x0100_0000;

/// How much a two-color split must cut a cell's squared error (summed over
/// its pixels and channels) to be drawn instead of one color.
const MIN_GAIN: u32 = 3 * 4 * 8 * 8;

/// Pixels at or below this alpha count as transparent.
const ALPHA_CUTOFF: u8 = 127;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub glyph: u32,
    /// `0x00RRGGBB`, or `DEFAULT_COLOR`.
    pub fg: u32,
    pub bg: u32,
}

/// Which block glyphs a picture is drawn with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, clap::ValueEnum)]
pub enum Glyphs {
    /// Quadrants: four pixels a cell, the sharpest.
    #[default]
    Quad,
    /// Half-blocks: two pixels a cell, for fonts that draw quadrants badly.
    Half,
}

/// The block glyph whose lit quarters are `mask`: bit 0 top left, bit 1 top
/// right, bit 2 bottom left, bit 3 bottom right.
const QUADRANTS: [u32; 16] = [
    0x0020, 0x2598, 0x259D, 0x2580, 0x2596, 0x258C, 0x259E, 0x259B, 0x2597, 0x259A, 0x2590, 0x259C,
    0x2584, 0x2599, 0x259F, 0x2588,
];

/// The picture as cells, row-major, in the chosen glyphs.
pub fn cells_with(image: &RgbaImage, cell_box: CellBox, glyphs: Glyphs) -> Vec<Cell> {
    match glyphs {
        Glyphs::Quad => quadrant_cells(image, cell_box),
        Glyphs::Half => cells(image, cell_box),
    }
}

/// Scales `image` to two by two pixels a cell and draws each cell as the
/// quadrant glyph whose split of its four pixels into two colors loses the
/// least. Half-blocks are two of those splits, so this never does worse.
pub fn quadrant_cells(image: &RgbaImage, cell_box: CellBox) -> Vec<Cell> {
    let scaled = imageops::resize(
        image,
        cell_box.columns * 2,
        cell_box.rows * 2,
        FilterType::Lanczos3,
    );

    let mut out = Vec::with_capacity((cell_box.columns * cell_box.rows) as usize);
    for row in 0..cell_box.rows {
        for column in 0..cell_box.columns {
            let quarter = |dx: u32, dy: u32| color(scaled.get_pixel(column * 2 + dx, row * 2 + dy));
            out.push(best_split([
                quarter(0, 0),
                quarter(1, 0),
                quarter(0, 1),
                quarter(1, 1),
            ]));
        }
    }
    out
}

/// The cell for four pixels (top left, top right, bottom left, bottom
/// right), `None` where transparent.
fn best_split(pixels: [Option<u32>; 4]) -> Cell {
    let lit = (0..4)
        .filter(|&i| pixels[i].is_some())
        .fold(0, |mask, i| mask | 1 << i);
    match lit {
        0 => {
            return Cell {
                glyph: SPACE,
                fg: DEFAULT_COLOR,
                bg: DEFAULT_COLOR,
            };
        }
        15 => {}
        // The terminal's background shows through: draw the rest in its mean color.
        _ => {
            let fg = mean(pixels.iter().flatten().copied());
            return Cell {
                glyph: QUADRANTS[lit],
                fg,
                bg: DEFAULT_COLOR,
            };
        }
    }

    let colors = pixels.map(|pixel| pixel.unwrap_or_default());
    let one = mean(colors.iter().copied());
    let flat_error: u32 = colors.iter().map(|&c| distance(c, one)).sum();
    let mut best = (
        flat_error,
        Cell {
            glyph: SPACE,
            fg: DEFAULT_COLOR,
            bg: one,
        },
    );

    // Masks 1 to 7 are each two-color split once (8 to 14 repeat them).
    for (mask, &glyph) in QUADRANTS.iter().enumerate().take(8).skip(1) {
        let is_lit = |i: usize| mask >> i & 1 == 1;
        let fg = mean((0..4).filter(|&i| is_lit(i)).map(|i| colors[i]));
        let bg = mean((0..4).filter(|&i| !is_lit(i)).map(|i| colors[i]));
        let error: u32 = (0..4)
            .map(|i| distance(colors[i], if is_lit(i) { fg } else { bg }))
            .sum();
        if error < best.0 {
            best = (error, Cell { glyph, fg, bg });
        }
    }

    // A split that barely beats one color is resampling noise, not an edge.
    if flat_error - best.0 < MIN_GAIN {
        return Cell {
            glyph: SPACE,
            fg: DEFAULT_COLOR,
            bg: one,
        };
    }
    best.1
}

/// The mean of `0x00RRGGBB` colors, per channel; 0 for none.
fn mean(colors: impl Iterator<Item = u32>) -> u32 {
    let (mut sum, mut n) = ([0u32; 3], 0u32);
    for color in colors {
        for (channel, total) in sum.iter_mut().enumerate() {
            *total += color >> (16 - 8 * channel) & 0xff;
        }
        n += 1;
    }
    if n == 0 {
        return 0;
    }
    sum.iter().enumerate().fold(0, |out, (channel, total)| {
        out | ((total + n / 2) / n) << (16 - 8 * channel)
    })
}

fn distance(a: u32, b: u32) -> u32 {
    (0..3)
        .map(|channel| {
            let shift = 16 - 8 * channel;
            (((a >> shift) & 0xff) as i32 - ((b >> shift) & 0xff) as i32).pow(2) as u32
        })
        .sum()
}

/// Scales `image` to the box (two pixels per cell, stacked) and turns each
/// pair of pixels into a cell, row-major.
pub fn cells(image: &RgbaImage, cell_box: CellBox) -> Vec<Cell> {
    let scaled = imageops::resize(
        image,
        cell_box.columns,
        cell_box.rows * 2,
        FilterType::Triangle,
    );

    let mut out = Vec::with_capacity((cell_box.columns * cell_box.rows) as usize);
    for row in 0..cell_box.rows {
        for column in 0..cell_box.columns {
            let upper = color(scaled.get_pixel(column, row * 2));
            let lower = color(scaled.get_pixel(column, row * 2 + 1));
            out.push(match (upper, lower) {
                (Some(fg), Some(bg)) => Cell {
                    glyph: UPPER_HALF,
                    fg,
                    bg,
                },
                (Some(fg), None) => Cell {
                    glyph: UPPER_HALF,
                    fg,
                    bg: DEFAULT_COLOR,
                },
                (None, Some(fg)) => Cell {
                    glyph: LOWER_HALF,
                    fg,
                    bg: DEFAULT_COLOR,
                },
                (None, None) => Cell {
                    glyph: SPACE,
                    fg: DEFAULT_COLOR,
                    bg: DEFAULT_COLOR,
                },
            });
        }
    }
    out
}

fn color(pixel: &Rgba<u8>) -> Option<u32> {
    let [r, g, b, a] = pixel.0;
    (a > ALPHA_CUTOFF).then_some((r as u32) << 16 | (g as u32) << 8 | b as u32)
}

/// Packs cells the way Claude Code's `Raster` element takes them: padded
/// base64 of little-endian u32 triplets `[codePoint, foreground, background]`.
pub fn raster_base64(cells: &[Cell]) -> String {
    let mut bytes = Vec::with_capacity(cells.len() * 12);
    for cell in cells {
        for word in [cell.glyph, cell.fg, cell.bg] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
    }
    STANDARD.encode(bytes)
}

/// How many colors the terminal shows, for ANSI output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Palette {
    TrueColor,
    Ansi256,
}

impl Palette {
    /// Truecolor, unless this is Apple's Terminal and `COLORTERM` does not
    /// say otherwise; `--colors` overrides it.
    pub fn detect() -> Self {
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
        if colorterm.contains("truecolor")
            || colorterm.contains("24bit")
            || program != "Apple_Terminal"
        {
            Palette::TrueColor
        } else {
            Palette::Ansi256
        }
    }
}

/// Renders cells as ANSI escape sequences, one line per row, colors reset at
/// each line's end.
pub fn ansi(cells: &[Cell], columns: u32, palette: Palette) -> String {
    let mut out = String::new();
    for row in cells.chunks(columns.max(1) as usize) {
        for cell in row {
            out.push_str("\x1b[0");
            push_color(&mut out, cell.fg, 38, palette);
            push_color(&mut out, cell.bg, 48, palette);
            out.push('m');
            out.push(char::from_u32(cell.glyph).unwrap_or(' '));
        }
        out.push_str("\x1b[0m\n");
    }
    out
}

fn push_color(out: &mut String, color: u32, layer: u8, palette: Palette) {
    if color == DEFAULT_COLOR {
        return;
    }
    let (r, g, b) = ((color >> 16) as u8, (color >> 8) as u8, color as u8);
    let _ = match palette {
        Palette::TrueColor => write!(out, ";{layer};2;{r};{g};{b}"),
        Palette::Ansi256 => write!(out, ";{layer};5;{}", to_256(r, g, b)),
    };
}

/// The nearest xterm-256 color: the 6x6x6 cube or the gray ramp.
fn to_256(r: u8, g: u8, b: u8) -> u8 {
    let level = |v: u8| {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            (v - 35) / 40
        }
    };
    let cube_value = |l: u8| if l == 0 { 0 } else { 55 + l * 40 };
    let (lr, lg, lb) = (level(r), level(g), level(b));
    let cube = 16 + 36 * lr + 6 * lg + lb;
    let cube_err = dist((r, g, b), (cube_value(lr), cube_value(lg), cube_value(lb)));

    let mean = ((r as u32 + g as u32 + b as u32) / 3) as u8;
    let gray_step = if mean > 238 {
        23
    } else {
        mean.saturating_sub(3) / 10
    };
    let gray_value = 8 + gray_step * 10;
    let gray_err = dist((r, g, b), (gray_value, gray_value, gray_value));

    if gray_err < cube_err {
        232 + gray_step
    } else {
        cube
    }
}

fn dist(a: (u8, u8, u8), b: (u8, u8, u8)) -> u32 {
    let d = |x: u8, y: u8| (x as i32 - y as i32).pow(2) as u32;
    d(a.0, b.0) + d(a.1, b.1) + d(a.2, b.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red_over_blue() -> RgbaImage {
        RgbaImage::from_raw(1, 2, vec![255, 0, 0, 255, /* lower */ 0, 0, 255, 255]).unwrap()
    }

    #[test]
    fn upper_pixel_is_foreground_lower_is_background() {
        let out = cells(
            &red_over_blue(),
            CellBox {
                columns: 1,
                rows: 1,
            },
        );
        assert_eq!(
            out,
            vec![Cell {
                glyph: UPPER_HALF,
                fg: 0xff0000,
                bg: 0x0000ff
            }]
        );
    }

    #[test]
    fn transparent_pixels_use_the_terminal_default() {
        let image = RgbaImage::from_raw(1, 2, vec![0, 0, 0, 0, 0, 255, 0, 255]).unwrap();
        let out = cells(
            &image,
            CellBox {
                columns: 1,
                rows: 1,
            },
        );
        assert_eq!(
            out,
            vec![Cell {
                glyph: LOWER_HALF,
                fg: 0x00ff00,
                bg: DEFAULT_COLOR
            }]
        );
    }

    #[test]
    fn raster_is_little_endian_triplets() {
        let cell = Cell {
            glyph: UPPER_HALF,
            fg: 0xff8800,
            bg: DEFAULT_COLOR,
        };
        let decoded = STANDARD.decode(raster_base64(&[cell])).unwrap();
        assert_eq!(decoded, [0x80, 0x25, 0, 0, 0x00, 0x88, 0xff, 0, 0, 0, 0, 1]);
    }

    #[test]
    fn ansi_truecolor_line() {
        let out = cells(
            &red_over_blue(),
            CellBox {
                columns: 1,
                rows: 1,
            },
        );
        assert_eq!(
            ansi(&out, 1, Palette::TrueColor),
            "\x1b[0;38;2;255;0;0;48;2;0;0;255m▀\x1b[0m\n"
        );
    }

    fn quad(pixels: [u32; 4]) -> Cell {
        best_split(pixels.map(Some))
    }

    #[test]
    fn quadrants_split_columns_and_diagonals() {
        let (red, blue) = (0xff0000, 0x0000ff);
        assert_eq!(
            quad([red, blue, red, blue]),
            Cell {
                glyph: 0x258C,
                fg: red,
                bg: blue
            }
        );
        assert_eq!(
            quad([red, blue, blue, red]),
            Cell {
                glyph: 0x259E,
                fg: blue,
                bg: red
            }
        );
        assert_eq!(
            quad([red, red, blue, blue]),
            Cell {
                glyph: 0x2580,
                fg: red,
                bg: blue
            }
        );
    }

    #[test]
    fn one_color_is_a_space_on_that_background() {
        assert_eq!(
            quad([0x123456; 4]),
            Cell {
                glyph: SPACE,
                fg: DEFAULT_COLOR,
                bg: 0x123456
            }
        );
    }

    #[test]
    fn transparent_quarters_show_the_terminal() {
        let cell = best_split([None, Some(0x00ff00), Some(0x00ff00), Some(0x00ff00)]);
        assert_eq!(
            cell,
            Cell {
                glyph: 0x259F,
                fg: 0x00ff00,
                bg: DEFAULT_COLOR
            }
        );
    }

    #[test]
    fn quadrant_picture_has_one_cell_per_box_cell() {
        let image = RgbaImage::from_fn(40, 20, |x, _| {
            if x < 20 {
                Rgba([255, 0, 0, 255])
            } else {
                Rgba([0, 0, 255, 255])
            }
        });
        let out = quadrant_cells(
            &image,
            CellBox {
                columns: 4,
                rows: 2,
            },
        );
        assert_eq!(out.len(), 8);
        // The left half is red: one color, no glyph, give or take resampling's rounding.
        assert_eq!((out[0].glyph, out[0].bg >> 16), (SPACE, 0xff));
        assert!(out[0].bg & 0xffff < 4);
    }

    #[test]
    fn ansi_256_picks_cube_and_grays() {
        assert_eq!(to_256(255, 0, 0), 196);
        assert_eq!(to_256(0, 0, 0), 16);
        assert_eq!(to_256(128, 128, 128), 244);
    }
}
