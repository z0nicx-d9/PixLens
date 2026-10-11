import { atom, memberOf, read, update } from 'claude-code'
import type { Elements, EngineInterface, PluginOptions, Register, RenderElement, RenderInput } from 'claude-code'

import type { Preview } from '../types'
import type { CellBox } from './lib'
import {
  baseName,
  cellRuns,
  centered,
  fitCells,
  imagePaths,
  imageTags,
  isImagePath,
  pastedFile,
  pngSize,
  projectSlug,
  shrinkTo,
  splitArgs,
  blankRows,
  tileLimit,
  withTag,
} from './lib'

type Kind = 'image' | 'blocks'
type Terminal = Elements['terminal']

type Settings = {
  binary: string
  mode: Kind | 'auto'
  glyphs: 'quad' | 'half'
  maxColumns: number
  maxRows: number
  thumbRows: number
  isAutomatic: boolean
  hasPasteThumbnails: boolean
  hasTool: boolean
  hasOverlay: boolean
}

const TOOL = 'image_preview'
const TOOL_ID = 'mcp__pixlens__image_preview'
const INSTALL_HINT = 'pixlens CLI not found: install it, or set its path in /config'
const NO_PREVIEW = 'no preview'
const USAGE = [
  'Usage:',
  '  /pixlens <image> [more]     show images in the transcript',
  '  /pixlens open <image|#n>    open in the default image viewer',
  '  /pixlens copy <image|#n>    copy to the clipboard (then ctrl+v)',
  'Click the name under a picture for a menu: Preview, Quick Look, copy, Finder.',
].join('\n')

/** Images per tool call; a command listing a folder of pictures shows the first few. */
const MAX_PER_CALL = 4

/**
 * How often the draft is read for `[Image #n]` tags. A paste changes the
 * draft without an edit the mod could hook, so it is read on a timer.
 */
const POLL_MS = 250

const previews = atom({ plugin: 'pixlens', key: 'previews' } as const, [])
const commands = atom({ plugin: 'pixlens', key: 'commands' } as const, [])
const pasted = atom({ plugin: 'pixlens', key: 'pasted' } as const, [])

/** What `auto` resolved to, once per load. */
let detectedKind: Kind | undefined
let hasWarned = false
/** Whether Pixlens Overlay runs, so block pictures carry its tags; asked once. */
let overlayRuns: { at: number; ready: Promise<boolean> } | undefined
const OVERLAY_RECHECK_MS = 10_000

/** The terminal and the band above the prompt as last drawn; they size pictures. */
let screen = { columns: 100, rows: 30 }
let band = { columns: 100, rows: 12 }
/** The tags and band size the thumbnails were last made for. */
let pastedKey = ''
let isPolling = false
let pasteDirFound: { sessionId: string; dir: string } | undefined

export const register: Register = (on, options) => {
  const settings = settingsFrom(options)

  on('session.start', async ($, e, next) => {
    await $.command.register({
      name: 'pixlens',
      description: 'Show, open or copy images',
      argumentHint: '[open|copy] <image|#n>',
    })
    if (settings.hasTool) {
      await $.tool.register({
        name: TOOL,
        description:
          'Shows image files (PNG, JPEG, GIF, WebP, BMP, TIFF, SVG) to the user inline in their terminal. ' +
          'Use it when the user wants to see an image or a chart you made. Only the user sees it; ' +
          'use Read when you need to look at the image yourself.',
        inputSchema: {
          type: 'object',
          properties: {
            paths: {
              type: 'array',
              items: { type: 'string' },
              minItems: 1,
              maxItems: MAX_PER_CALL,
              description: 'Image files, absolute or relative to the working directory',
            },
          },
          required: ['paths'],
        },
        isDeferred: false,
      })
    }
    if (settings.hasPasteThumbnails) $.clock.every(POLL_MS, () => pollPaste($, settings))
    return next(e)
  })

  on('tool.call', { tool: TOOL_ID }, async ($, e) => {
    const paths = Array.isArray(e.paths) ? e.paths.filter((p): p is string => typeof p === 'string') : []
    if (paths.length === 0) return { deny: 'paths must list at least one image file' }

    const list = await collect($, settings, paths)
    if (list.length === 0) return { deny: `no such file: ${paths.join(', ')}` }

    await keep($, e.tool_use_id, list)
    return { result: `Shown to the user:\n${summary(list, paths.length).join('\n')}` }
  }).catch(($, e, next) => (next.called ? next(e) : { deny: 'pixlens failed to show the image' }))

  if (settings.isAutomatic) {
    on('tool.call', { tool: 'Read' }, async ($, e, next) => {
      const ran = await next(e)
      if (ran.deny === undefined && !ran.isError && isImagePath(e.file_path)) {
        await keep($, e.tool_use_id, await collect($, settings, [e.file_path]))
      }
      return ran
    }).catch(($, e, next) => next(e))

    on('tool.call', { tool: 'Write' }, async ($, e, next) => {
      const ran = await next(e)
      if (ran.deny === undefined && !ran.isError && isImagePath(e.file_path)) {
        await keep($, e.tool_use_id, await collect($, settings, [e.file_path]))
      }
      return ran
    }).catch(($, e, next) => next(e))

    on('tool.call', { tool: 'Edit' }, async ($, e, next) => {
      const ran = await next(e)
      if (ran.deny === undefined && !ran.isError && isImagePath(e.file_path)) {
        await keep($, e.tool_use_id, await collect($, settings, [e.file_path]))
      }
      return ran
    }).catch(($, e, next) => next(e))

    on('tool.call', { tool: 'Bash' }, async ($, e, next) => {
      const startedAt = await $.clock.now()
      const ran = await next(e)
      if (ran.deny !== undefined || ran.isError) return ran

      const output = (ran.result as { stdout?: unknown } | undefined)?.stdout
      const named = imagePaths(`${e.command}\n${typeof output === 'string' ? output : ''}`, 12)
      if (named.length > 0) await keep($, e.tool_use_id, await collect($, settings, named, startedAt))
      return ran
    }).catch(($, e, next) => next(e))
  }

  on('command.run', { command: 'pixlens' }, async ($, e) => {
    const [first, ...rest] = splitArgs(e.args)
    if (first === undefined) return { text: USAGE }

    if (first === 'open' || first === 'copy') {
      if (rest[0] === undefined) return { text: USAGE }
      const path = await target($, rest[0])
      if (path === undefined) return { text: `pixlens: no image ${rest[0]}` }
      const name = rest[0].startsWith('#') ? rest[0] : baseName(path)
      return {
        text:
          first === 'open'
            ? await runCli($, settings, ['open', path], `Opened ${name}`)
            : await runCli($, settings, ['copy', path], `Copied ${name}: ctrl+v pastes it`),
      }
    }

    const paths = [first, ...rest]
    const list = await collect($, settings, paths)
    await update($, commands, all => [...all.filter(c => c.args !== e.args), { args: e.args, previews: list }].slice(-20))
    return { text: summary(list, paths.length).join('\n') }
  })

  on('ui.render', { component: 'AbovePrompt' }, async ($, e, next) => {
    if (e.surface !== 'terminal' || e.props.hasSurvey) return next(e)
    band = { columns: e.props.bodyColumns, rows: e.props.maxRows }
    if (e.viewport !== undefined) screen = { columns: e.viewport.columns, rows: e.viewport.rows }
    const list = await read($, pasted)
    if (list.length === 0) return next(e)

    const els = $.ui.resolve(e)
    const { Box } = els
    const below = await next(e)
    return (
      <Box flexDirection="column">
        <Box flexDirection="row" columnGap={2}>
          {list.map((p, i) => tile(els, p, `paste-${i}`, 255, () => openFile($, settings, p.path), true))}
        </Box>
        {below}
      </Box>
    )
  })

  on('ui.render', { component: 'ToolResult' }, async ($, e, next) => {
    if (e.surface !== 'terminal') return next(e)
    const list = await read($, memberOf(previews, e))
    if (list.length === 0) return next(e)
    return drawBelow($, settings, e, await next(e), list)
  })

  on('ui.render', { component: 'ToolGroup' }, async ($, e, next) => {
    if (e.surface !== 'terminal') return next(e)
    const ids = e.props.calls.flatMap(call => (call.tool_use_id === undefined ? [] : [call.tool_use_id]))
    const lists = await Promise.all(ids.map(id => $.state.get({ plugin: 'pixlens', key: 'previews', id })))
    const list = lists.flatMap(({ value }) => value ?? [])
    if (list.length === 0) return next(e)
    return drawBelow($, settings, e, await next(e), list)
  })

  on('ui.render', { component: 'CommandOutput', props: { command: 'pixlens' } }, async ($, e, next) => {
    if (e.surface !== 'terminal') return next(e)
    const runs = await read($, commands)
    const list = runs.findLast(run => run.args === e.props.args)?.previews ?? []
    if (list.length === 0) return next(e)
    return drawBelow($, settings, e, await next(e), list, 0)
  })
}

/**
 * What a click on a picture's name does: a menu at the pointer (open in
 * Preview, Quick Look, copy, show in Finder) where Pixlens Overlay is
 * installed, else the picture opens in the default viewer. Says so only
 * when it fails.
 */
async function openFile($: EngineInterface, settings: Settings, path: string): Promise<void> {
  const done = `Opened ${baseName(path)}`
  const message = await runCli($, settings, ['menu', path, '--app', overlayApp($)], done)
  if (message !== done) await $.ui.toast(`pixlens: ${message}`)
}

/** Runs a `pixlens` subcommand that answers JSON: `done` when it worked, else why not. */
async function runCli($: EngineInterface, settings: Settings, args: string[], done: string): Promise<string> {
  try {
    const { stdout } = await $.process.run([settings.binary, ...args], { timeoutMs: 20_000 })
    const answer = JSON.parse(stdout) as { error?: string }
    return answer.error ?? done
  } catch {
    return INSTALL_HINT
  }
}

/** A `/pixlens` argument as a file: `#n` is the image pasted as `[Image #n]`. */
async function target($: EngineInterface, arg: string): Promise<string | undefined> {
  const tag = /^#(\d+)$/.exec(arg)
  if (tag === null) {
    const path = await absolute($, arg)
    const stat = await $.fs.stat(path, { resolve: true }).catch(() => undefined)
    return stat?.kind === 'file' ? (stat.realPath ?? path) : undefined
  }
  const dir = await pasteDir($)
  if (dir === undefined) return undefined
  const name = pastedFile((await $.fs.list(dir).catch(() => [])).map(entry => entry.name), Number(tag[1]))
  return name === undefined ? undefined : `${dir}/${name}`
}

/**
 * Reads the draft and, when its `[Image #n]` tags or the band's size
 * changed, makes thumbnails of those pastes. A tag whose file is not on
 * disk yet is looked for again on the next tick.
 */
async function pollPaste($: EngineInterface, settings: Settings): Promise<void> {
  if (isPolling) return
  isPolling = true
  try {
    const numbers = imageTags((await $.prompt.read()).text)
    const key = numbers.length === 0 ? '' : `${numbers.join(',')}|${band.columns}x${band.rows}`
    if (key === pastedKey) return

    const list = numbers.length === 0 ? [] : await pastedPreviews($, settings, numbers)
    pastedKey = list.some(p => p.error === NO_PREVIEW) ? 'retry' : key
    await $.state.set({ plugin: 'pixlens', key: 'pasted' }, list)
  } finally {
    isPolling = false
  }
}

async function pastedPreviews($: EngineInterface, settings: Settings, numbers: number[]): Promise<Preview[]> {
  const dir = await pasteDir($)
  const names = dir === undefined ? [] : (await $.fs.list(dir).catch(() => [])).map(entry => entry.name)
  const limit = tileLimit(numbers.length, band.columns, band.rows, settings.thumbRows)

  const list: Preview[] = []
  for (const n of numbers) {
    const name = pastedFile(names, n)
    if (dir === undefined || name === undefined) {
      list.push({ ...failed('', NO_PREVIEW), name: `#${n}`, columns: Math.min(limit.columns, 10), rows: Math.min(limit.rows, 2) })
      continue
    }
    list.push({ ...(await preview($, settings, `${dir}/${name}`, limit)), name: `#${n}` })
  }
  return list
}

/**
 * The folder Claude Code keeps this session's pastes in:
 * `<tmp>/claude-<uid>/<project>/<session>/images`. The project part is the
 * folder the session started in, spelled as `projectSlug` does; when that
 * misses (the folder moved), every project folder is tried for the session.
 */
async function pasteDir($: EngineInterface): Promise<string | undefined> {
  const sessionId = await $.session.id()
  if (pasteDirFound?.sessionId === sessionId) return pasteDirFound.dir

  const override = await $.env.get('CLAUDE_CODE_TMPDIR')
  const bases = [...(override === undefined ? [] : [override]), '/tmp', '/private/tmp']
  const roots: string[] = []
  for (const base of bases) {
    roots.push(base)
    for (const entry of await $.fs.list(base).catch(() => [])) {
      if (entry.kind === 'dir' && entry.name.startsWith('claude-')) roots.push(`${base}/${entry.name}`)
    }
  }

  const project = projectSlug(await $.session.root())
  for (const root of roots) {
    const dir = `${root}/${project}/${sessionId}/images`
    if (await $.fs.exists(dir)) {
      pasteDirFound = { sessionId, dir }
      return dir
    }
  }
  for (const root of roots) {
    for (const entry of await $.fs.list(root).catch(() => [])) {
      const dir = `${root}/${entry.name}/${sessionId}/images`
      if (entry.kind === 'dir' && (await $.fs.exists(dir))) {
        pasteDirFound = { sessionId, dir }
        return dir
      }
    }
  }
  return undefined
}

/**
 * Pixels where the terminal draws them (kitty, Ghostty, or Claude Code told
 * to with CLAUDE_CODE_FORCE_TERMINAL_IMAGES), block glyphs elsewhere.
 */
async function drawKind($: EngineInterface, settings: Settings): Promise<Kind> {
  if (settings.mode !== 'auto') return settings.mode
  if (detectedKind !== undefined) return detectedKind

  const [forced, tmux, term, program, kitty] = await Promise.all([
    $.env.get('CLAUDE_CODE_FORCE_TERMINAL_IMAGES'),
    $.env.get('TMUX'),
    $.env.get('TERM'),
    $.env.get('TERM_PROGRAM'),
    $.env.get('KITTY_WINDOW_ID'),
  ])
  const hasGraphics = Boolean(kitty) || term === 'xterm-kitty' || program === 'ghostty'
  detectedKind = forced === '1' || (!tmux && hasGraphics) ? 'image' : 'blocks'
  return detectedKind
}

async function absolute($: EngineInterface, path: string): Promise<string> {
  if (!path.startsWith('~/')) return path
  const home = await $.env.get('HOME')
  return home === undefined ? path : `${home}${path.slice(1)}`
}

/**
 * Previews of the files among `paths` that exist; with `since`, only those
 * written since then (what a shell command made, not every path it named),
 * and only those that drew.
 */
async function collect($: EngineInterface, settings: Settings, paths: string[], since?: number): Promise<Preview[]> {
  const limit = { columns: settings.maxColumns, rows: settings.maxRows }
  const found: Preview[] = []
  for (const raw of paths) {
    if (found.length === MAX_PER_CALL) break
    const path = await absolute($, raw)
    const stat = await $.fs.stat(path).catch(() => undefined)
    if (stat?.kind !== 'file') continue
    if (since !== undefined && stat.mtimeMs < since - 1000) continue
    const shown = await preview($, settings, path, limit)
    if (shown.error === undefined || since === undefined) found.push(shown)
  }
  return found
}

/** Runs `pixlens render` for one file; a missing CLI falls back to drawing a PNG as it is. */
async function preview($: EngineInterface, settings: Settings, path: string, limit: CellBox): Promise<Preview> {
  const drawAs = await drawKind($, settings)
  const argv = [
    settings.binary,
    'render',
    path,
    '--columns',
    String(limit.columns),
    '--rows',
    String(limit.rows),
    ...(drawAs === 'image' ? ['--png'] : ['--cells', '--glyphs', settings.glyphs]),
    ...(drawAs === 'blocks' && (await hasOverlay($, settings)) ? ['--overlay'] : []),
  ]

  let stdout: string
  try {
    stdout = (await $.process.run(argv, { timeoutMs: 20_000 })).stdout
  } catch {
    return withoutCli($, path, drawAs, limit)
  }

  try {
    const answer = JSON.parse(stdout) as Partial<Preview>
    if (answer.error !== undefined) return failed(path, answer.error)
    return {
      path: answer.path ?? path,
      name: baseName(path),
      width: answer.width ?? 0,
      height: answer.height ?? 0,
      columns: answer.columns ?? 1,
      rows: answer.rows ?? 1,
      png: answer.png,
      cells: answer.cells,
      ...(answer.tag === undefined ? {} : { tag: answer.tag }),
    }
  } catch {
    return failed(path, `unexpected output from ${settings.binary}`)
  }
}

/**
 * Starts Pixlens Overlay (macOS), which lays the real picture over each
 * tagged picture in Apple's Terminal, and says whether it is ready to draw:
 * false where it cannot run or may not read the screen. Asked again after
 * a while, so an overlay that stopped is noticed.
 */
async function hasOverlay($: EngineInterface, settings: Settings): Promise<boolean> {
  if (!settings.hasOverlay) return false
  const now = await $.clock.now()
  if (overlayRuns === undefined || now - overlayRuns.at > OVERLAY_RECHECK_MS) {
    const ready = $.process
      .run([settings.binary, 'overlay', '--app', overlayApp($)], { timeoutMs: 5_000 })
      .then(({ stdout }) => (JSON.parse(stdout) as { ready?: boolean }).ready === true)
      .catch(() => false)
    overlayRuns = { at: now, ready }
  }
  return overlayRuns.ready
}

/** Pixlens Overlay, which this plugin ships (built for macOS). */
function overlayApp($: EngineInterface): string {
  return `${$.plugin.root}/overlay/Pixlens Overlay.app`
}

/** Without the CLI only a PNG on a pixel terminal can be drawn: the terminal decodes it. */
async function withoutCli($: EngineInterface, path: string, drawAs: Kind, limit: CellBox): Promise<Preview> {
  if (!hasWarned) {
    hasWarned = true
    await $.ui.toast(`pixlens: ${INSTALL_HINT}`)
  }
  if (drawAs !== 'image' || !/\.png$/i.test(path)) return failed(path, INSTALL_HINT)

  try {
    const stat = await $.fs.stat(path, { resolve: true })
    const { base64 } = await $.fs.read(path, { as: 'bytes' })
    const size = pngSize(base64)
    if (size === undefined || stat.realPath === undefined) return failed(path, 'not a PNG file')
    return {
      path: stat.realPath,
      name: baseName(path),
      ...size,
      ...fitCells(size.width, size.height, limit.columns, limit.rows),
      png: stat.realPath,
    }
  } catch {
    return failed(path, INSTALL_HINT)
  }
}

async function keep($: EngineInterface, toolUseId: string, list: Preview[]): Promise<void> {
  if (list.length > 0) await $.state.set({ plugin: 'pixlens', key: 'previews', id: toolUseId }, list)
}

/**
 * The engine's own drawing with a row of tiles under it, wrapping where the
 * viewport is too narrow for them side by side.
 */
function drawBelow(
  $: EngineInterface,
  settings: Settings,
  e: RenderInput<'ToolResult' | 'ToolGroup' | 'CommandOutput', 'terminal'>,
  own: RenderElement,
  list: readonly Preview[],
  indent = 5,
): RenderElement {
  if (e.viewport !== undefined) screen = { columns: e.viewport.columns, rows: e.viewport.rows }
  const els = $.ui.resolve(e)
  const { Box } = els
  const room = Math.max(8, screen.columns - indent - 4)

  return (
    <Box flexDirection="column">
      {own}
      <Box marginLeft={indent} flexDirection="row" flexWrap="wrap" columnGap={1}>
        {list.map((p, i) => tile(els, p, `pixlens-${i}`, room, () => openFile($, settings, p.path)))}
      </Box>
    </Box>
  )
}

/**
 * One framed picture with its caption under it. Block pictures are drawn as
 * colored text inside one button with the caption, so a click anywhere on
 * them opens the file; a pixel picture (`Image`) cannot sit in a button, so
 * there the caption alone is the button. A pixel picture shrinks to `room`.
 */
/**
 * One picture with its name under it, framed. `isCompact` drops the frame,
 * for the band above the prompt: every row it takes pushes the screen up.
 */
function tile(els: Terminal, p: Preview, key: string, room: number, onOpen: () => unknown, isCompact = false): RenderElement {
  const { Box, Text, Image, Button } = els
  const box = p.png === undefined ? p : shrinkTo(p, room)
  const caption = p.width > 0 && !p.name.startsWith('#') ? `${p.name} · ${p.width}×${p.height}` : p.name
  const inner = p.error === undefined ? Math.max(box.columns, Math.min(caption.length, 20)) : Math.max(box.columns, 10)
  const frame = isCompact
    ? ({ flexDirection: 'column', alignItems: 'center', width: inner } as const)
    : ({ flexDirection: 'column', alignItems: 'center', width: inner + 2, borderStyle: 'round', borderDimColor: true } as const)

  if (p.error !== undefined) {
    return (
      <Box {...frame}>
        <Box width={inner} height={box.rows} alignItems="center" justifyContent="center">
          <Text dimColor wrap="truncate">
            {p.error}
          </Text>
        </Box>
        <Text dimColor>{centered(caption, inner)}</Text>
      </Box>
    )
  }

  if (p.png !== undefined) {
    return (
      <Box {...frame}>
        <Image key={key} source={{ file: p.png, format: 'png' }} columns={box.columns} rows={box.rows} alt={`[${p.name}]`} />
        <Button key={key} label={centered(caption, inner)} plain dimColor onPress={onOpen} />
      </Box>
    )
  }

  // Plain text, not inside the Button: a Button inverts every cell under
  // the pointer, which garbles a picture. The name under it opens the file.
  const pad = ' '.repeat(Math.max(0, Math.floor((inner - p.columns) / 2)))
  return (
    <Box {...frame}>
      {/* Under a ready overlay the cells stay blank but for its tags: while the
          terminal redraws, the real picture can trail a frame behind, and an
          empty box shows less than a coarse copy of the picture. */}
      {(p.tag === undefined ? cellRuns(p.cells ?? '', p.columns, p.rows) : withTag(blankRows(p.columns, p.rows), p.tag)).map(runs => (
        <Text>
          {pad}
          {runs.map(run => (
            <Text
              {...(run.color === undefined ? {} : { color: run.color })}
              {...(run.backgroundColor === undefined ? {} : { backgroundColor: run.backgroundColor })}
              {...(run.isDim === true ? { dimColor: true } : {})}
            >
              {run.text}
            </Text>
          ))}
        </Text>
      ))}
      {p.tag === undefined ? (
        <Button key={key} label={centered(caption, inner)} plain dimColor onPress={onOpen} />
      ) : (
        // Pixlens Overlay lights and handles the name itself; a Button would
        // invert under the pointer on top of its ring. The Box keeps the
        // name centered once, under the picture's middle.
        <Box width={inner}>
          <Text dimColor>{centered(caption, inner)}</Text>
        </Box>
      )}
    </Box>
  )
}

/** One line per preview for what the model reads back. */
function summary(list: readonly Preview[], asked: number): string[] {
  const lines = list.map(p => (p.error ? `${p.name}: ${p.error}` : `${p.name} (${p.width}x${p.height})`))
  if (asked > list.length) lines.push(`${asked - list.length} path(s) not found`)
  return lines
}

function failed(path: string, error: string): Preview {
  return { path, name: baseName(path), width: 0, height: 0, columns: 1, rows: 1, error }
}

function settingsFrom(options: PluginOptions): Settings {
  const text = (key: string, fallback: string) => {
    const value = options[key]
    return typeof value === 'string' && value !== '' ? value : fallback
  }
  const count = (key: string, fallback: number) => {
    const value = Number(options[key])
    return Number.isFinite(value) && value >= 1 ? Math.min(255, Math.floor(value)) : fallback
  }
  const mode = text('mode', 'auto')

  return {
    binary: text('binary', 'pixlens'),
    mode: mode === 'image' || mode === 'blocks' ? mode : 'auto',
    glyphs: text('glyphs', 'quad') === 'half' ? 'half' : 'quad',
    maxColumns: count('maxColumns', 32),
    maxRows: count('maxRows', 9),
    thumbRows: count('thumbRows', 3),
    isAutomatic: options.automatic !== false,
    hasPasteThumbnails: options.pasteThumbnails !== false,
    hasTool: options.tool !== false,
    hasOverlay: options.overlay !== false,
  }
}
