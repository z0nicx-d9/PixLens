// Pure helpers: no `$`, so the tests call them directly.

/** What the pixlens CLI reads, by extension. */
export const EXTENSIONS = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp', 'tif', 'tiff', 'ico', 'qoi', 'svg', 'svgz']

const IMAGE_PATH = new RegExp(`\\.(?:${EXTENSIONS.join('|')})$`, 'i')

/** Characters that end a path in a command line or a tool's output. */
const SEPARATORS = /[\s'"`()<>=,;|&[\]{}]+/

/** Trailing punctuation a sentence puts after a path (`saved chart.png.`). */
const TRAILING = /[.:!?]+$/

export function isImagePath(path: string): boolean {
  return IMAGE_PATH.test(path)
}

/**
 * The image paths named in free text (a shell command, its output), in the
 * order they first appear, at most `limit`. URLs are left out.
 */
export function imagePaths(text: string, limit = 6): string[] {
  const found: string[] = []
  for (const raw of text.split(SEPARATORS)) {
    const token = raw.replace(TRAILING, '')
    if (token.includes('://') || !isImagePath(token) || found.includes(token)) continue
    found.push(token)
    if (found.length === limit) break
  }
  return found
}

/** Splits `/pixlens` arguments: words, or 'quoted' / "quoted" paths with spaces. */
export function splitArgs(args: string): string[] {
  return [...args.matchAll(/"([^"]*)"|'([^']*)'|(\S+)/g)].map(m => m[1] ?? m[2] ?? m[3] ?? '').filter(Boolean)
}

/** The numbers of the `[Image #n]` tags in a draft, each once, in order. */
export function imageTags(draft: string): number[] {
  const numbers: number[] = []
  for (const match of draft.matchAll(/\[Image #(\d+)\]/g)) {
    const n = Number(match[1])
    if (!numbers.includes(n)) numbers.push(n)
  }
  return numbers
}

/** How Claude Code names a project's temp folder: the path, every non-alphanumeric a `-`. */
export function projectSlug(path: string): string {
  return path.replace(/[^A-Za-z0-9]/g, '-')
}

/** The pasted image file for tag `n` among a folder's names (`3.png`, `3.jpg`, ...). */
export function pastedFile(names: readonly string[], n: number): string | undefined {
  return names.find(name => name.slice(0, name.lastIndexOf('.')) === String(n) && isImagePath(name))
}

/**
 * The largest picture each of `count` tiles may hold in a row `columns`
 * wide, at most `maxColumns` x `thumbRows`: tiles sit two columns apart
 * and the label under each takes a row.
 */
export function tileLimit(count: number, columns: number, maxRows: number, thumbRows: number, maxColumns = 20): CellBox {
  const n = Math.max(1, count)
  const perTile = Math.floor((columns - 2 * (n - 1)) / n)
  return {
    columns: Math.max(4, Math.min(maxColumns, perTile)),
    rows: Math.max(1, Math.min(thumbRows, maxRows - 1)),
  }
}

/** The last path segment, for captions. */
export function baseName(path: string): string {
  return path.split('/').filter(Boolean).pop() ?? path
}

export type CellBox = { columns: number; rows: number }

/**
 * Fits a picture inside `maxColumns` x `maxRows` cells keeping its aspect
 * ratio, two square pixels to a cell, never above one pixel per half cell.
 * The same rule as the CLI's `fit_cells`.
 */
export function fitCells(width: number, height: number, maxColumns: number, maxRows: number): CellBox {
  const w = Math.max(1, width)
  const h = Math.max(1, height)
  const maxW = Math.max(1, maxColumns)
  const maxH = Math.max(1, maxRows) * 2
  const scale = Math.min(maxW / w, maxH / h, 1)
  return {
    columns: Math.min(maxW, Math.max(1, Math.round(w * scale))),
    rows: Math.min(Math.max(1, maxRows), Math.max(1, Math.ceil((h * scale) / 2))),
  }
}

/** Shrinks a box to at most `room` columns, rows in proportion. */
export function shrinkTo(box: CellBox, room: number): CellBox {
  if (box.columns <= room) return box
  const columns = Math.max(1, room)
  return { columns, rows: Math.max(1, Math.round((box.rows * columns) / box.columns)) }
}

/**
 * Width and height from the head of a PNG file given as base64 (its IHDR
 * chunk, the first 24 bytes); undefined when it is not a PNG.
 */
export function pngSize(base64: string): { width: number; height: number } | undefined {
  const head = atob(base64.slice(0, 32))
  if (head.length < 24 || head.slice(1, 4) !== 'PNG') return undefined
  const u32 = (at: number) =>
    ((head.charCodeAt(at) << 24) | (head.charCodeAt(at + 1) << 16) | (head.charCodeAt(at + 2) << 8) | head.charCodeAt(at + 3)) >>> 0
  return { width: u32(16), height: u32(20) }
}

/** A stretch of one row of a block picture drawn in one pair of colors. */
export type Run = { text: string; color?: string; backgroundColor?: string; isDim?: boolean }

/**
 * Turns the CLI's packed cells (base64 of little-endian u32 triplets
 * `[codePoint, foreground, background]`, row-major) into rows of runs, so
 * the picture can be drawn as colored text. `0x01000000` is the terminal's
 * own color and leaves the prop out.
 */
export function cellRuns(cells: string, columns: number, rows: number): Run[][] {
  const bytes = atob(cells)
  const word = (at: number) =>
    (bytes.charCodeAt(at) | (bytes.charCodeAt(at + 1) << 8) | (bytes.charCodeAt(at + 2) << 16) | (bytes.charCodeAt(at + 3) << 24)) >>> 0
  const hex = (color: number) => (color === 0x01000000 ? undefined : `#${color.toString(16).padStart(6, '0')}`)

  const out: Run[][] = []
  for (let row = 0; row < rows; row++) {
    const runs: Run[] = []
    for (let column = 0; column < columns; column++) {
      const at = (row * columns + column) * 12
      if (at + 12 > bytes.length) break
      const text = String.fromCharCode(word(at))
      const color = hex(word(at + 4))
      const backgroundColor = hex(word(at + 8))
      const last = runs.at(-1)
      if (last !== undefined && last.color === color && last.backgroundColor === backgroundColor) {
        last.text += text
      } else {
        runs.push({ text, ...(color === undefined ? {} : { color }), ...(backgroundColor === undefined ? {} : { backgroundColor }) })
      }
    }
    out.push(runs)
  }
  return out
}

/** `text` cut to `width` with an ellipsis, then centered in it. */
export function centered(text: string, width: number): string {
  const cut = text.length > width ? `${text.slice(0, Math.max(0, width - 1))}…` : text
  const left = Math.floor((width - cut.length) / 2)
  return ' '.repeat(Math.max(0, left)) + cut
}

/** Starts the tag on a picture's last row, where its first row is scrolled away. */
export const BOTTOM_TAG = '\u28FE'
/**
 * Starts every other row of a tagged picture, followed by the picture's id,
 * so the overlay can tell its rows from a pane's edge or another picture.
 */
export const ROW_MARK = '\u28FD'

/**
 * Puts Pixlens Overlay's tag in a picture's first cells, where the overlay
 * finds it on screen and lays the real picture over the box. The last row
 * carries the tag too, starting with BOTTOM_TAG, and the rows between carry
 * it starting with ROW_MARK. Those cells replace the picture's, drawn dim; without a
 * tag the rows are unchanged.
 */
export function withTag(rows: Run[][], tag: string | undefined): Run[][] {
  if (tag === undefined || rows.length === 0) return rows
  const id = [...tag].slice(1).join('')
  return rows.map((runs, row) => {
    if (row === 0) return replaceStart(runs, tag)
    return replaceStart(runs, (row === rows.length - 1 ? BOTTOM_TAG : ROW_MARK) + id)
  })
}

/** `runs` with their first cells replaced by `text`, one cell a character. */
function replaceStart(runs: Run[], text: string): Run[] {
  const rest: Run[] = []
  let skip = [...text].length
  for (const run of runs) {
    const chars = [...run.text]
    if (skip >= chars.length) {
      skip -= chars.length
      continue
    }
    rest.push({ ...run, text: chars.slice(skip).join('') })
    skip = 0
  }
  return [{ text, isDim: true }, ...rest]
}

/** A box of blank cells, `rows` rows of `columns`. */
export function blankRows(columns: number, rows: number): Run[][] {
  return Array.from({ length: rows }, () => [{ text: ' '.repeat(Math.max(0, columns)) }])
}
