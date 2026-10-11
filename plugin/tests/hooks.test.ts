import type { On, RenderElement } from 'claude-code'
import { expect, mock, test } from 'claude-code/testing'

const ENGINE_ROW: RenderElement = { type: 'engine', ref: 0 }
/** 40 x 15 cells, each an orange upper half-block on the default background. */
const CELLS = base64(new Uint8Array(Uint32Array.from({ length: 40 * 15 * 3 }, (_, i) => [0x2580, 0xff8800, 0x01000000][i % 3] ?? 0).buffer))

function base64(bytes: ArrayLike<number>): string {
  return btoa(String.fromCharCode(...Array.from(bytes)))
}

type Fake = { argv: string[][]; toolUseId: string; stdout: string }

/**
 * Stands in for the engine under the plugin: a working directory holding
 * `plot.png` (written at t=5000) and `old.png` (t=0), a `pixlens` CLI that
 * answers JSON, and tools that answer as told.
 */
function engine(on: On, variables: Record<string, string>, isCliInstalled = true): Fake {
  const fake: Fake = { argv: [], toolUseId: '', stdout: 'saved plot.png' }
  mock.env(on, { HOME: '/home/me', ...variables })
  mock.clock(on, { now: 4000 })

  on('fs.stat', ($, e) => {
    const name = e.path.split('/').pop() ?? ''
    const mtimeMs = { 'plot.png': 5000, 'old.png': 0 }[name]
    if (mtimeMs === undefined) return { deny: `ENOENT: ${e.path}` }
    return { value: { kind: 'file', size: 100, mtimeMs, isLink: false, realPath: `/work/${name}` } }
  })
  on('process.run', ($, e) => {
    if (e.argv[1] === 'overlay') return { deny: 'Pixlens Overlay.app is not installed' }
    fake.argv.push([...e.argv])
    if (!isCliInstalled) return { deny: 'spawn pixlens ENOENT' }
    const isPng = e.argv.includes('--png')
    const answer = {
      path: `/work/${e.argv[2]}`,
      format: 'png',
      width: 640,
      height: 480,
      columns: 40,
      rows: 15,
      ...(isPng ? { png: '/cache/pixlens/1.png' } : { cells: CELLS }),
    }
    return { value: { exitCode: 0, stdout: JSON.stringify(answer), stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
  })
  on('tool.call', ($, e) => {
    fake.toolUseId = e.tool_use_id
    return { result: { stdout: fake.stdout, stderr: '', interrupted: false } }
  })
  on('ui.render', () => ENGINE_ROW)
  on('ui.toast', () => ({ value: undefined }))
  return fake
}

function toolResult(id: string, columns = 100) {
  return {
    surface: 'terminal',
    component: 'ToolResult',
    requestId: id,
    viewport: { columns, rows: 40 },
    props: { tool_use_id: id, tool: 'Bash', output: {}, isErrored: false },
  } as const
}

/** Rows of the fake picture drawn: each is one run of orange text. */
function orangeRows(tree: RenderElement | string): number {
  return find(tree, 'Text').filter(t => (t.props as { color?: string }).color === '#ff8800').length
}

function find(tree: RenderElement | string, type: string): Extract<RenderElement, { props: unknown }>[] {
  if (typeof tree === 'string') return []
  const own = tree.type === type && 'props' in tree ? [tree as Extract<RenderElement, { props: unknown }>] : []
  const children = 'children' in tree && Array.isArray(tree.children) ? tree.children : []
  return [...own, ...children.flatMap(child => find(child as RenderElement | string, type))]
}

test('a Bash command that writes a PNG gets half-blocks under its result', async ($, on) => {
  const fake = engine(on, { TERM_PROGRAM: 'herdr' })

  await $.tool.call({ tool: 'Bash', command: 'python plot.py' })
  expect(fake.argv).toEqual([['pixlens', 'render', 'plot.png', '--columns', '32', '--rows', '9', '--cells', '--glyphs', 'quad']])

  const tree = await $.ui.render(toolResult(fake.toolUseId))
  expect(find(tree, 'Button').map(b => b.props)).toMatchObject([{ key: 'pixlens-0', plain: true }])
  expect(orangeRows(tree)).toBe(15)
  expect(JSON.stringify(tree)).toContain('"ref":0')
})

test('files a command only names, written before it ran, are not shown', async ($, on) => {
  const fake = engine(on, { TERM_PROGRAM: 'herdr' })
  fake.stdout = 'old.png'

  await $.tool.call({ tool: 'Bash', command: 'ls old.png' })
  expect(fake.argv).toEqual([])
  expect(await $.ui.render(toolResult(fake.toolUseId))).toEqual(ENGINE_ROW)
})

test('Ghostty gets real pixels, shrunk to a narrow viewport', async ($, on) => {
  const fake = engine(on, { TERM_PROGRAM: 'ghostty' })

  await $.tool.call({ tool: 'Read', file_path: 'plot.png' })
  expect(fake.argv[0]).toContain('--png')

  const [image] = find(await $.ui.render(toolResult(fake.toolUseId, 27)), 'Image')
  expect(image?.props).toEqual({
    key: 'pixlens-0',
    source: { file: '/cache/pixlens/1.png', format: 'png' },
    columns: 18,
    rows: 7,
    alt: '[plot.png]',
  })
})

test('inside tmux even Ghostty gets half-blocks', async ($, on) => {
  const fake = engine(on, { TERM_PROGRAM: 'ghostty', TMUX: '/tmp/tmux-1/default,1,0' })

  await $.tool.call({ tool: 'Read', file_path: 'plot.png' })
  expect(fake.argv[0]).toContain('--cells')
})

test('without the CLI a PNG still draws where the terminal decodes it', async ($, on) => {
  const fake = engine(on, { TERM_PROGRAM: 'ghostty' }, false)
  const head = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, 0, 0, 2, 0x80, 0, 0, 1, 0xe0]
  on('fs.read', () => ({ value: { base64: base64(head) } }))

  await $.tool.call({ tool: 'Read', file_path: 'plot.png' })
  const [image] = find(await $.ui.render(toolResult(fake.toolUseId)), 'Image')
  expect(image?.props).toMatchObject({ source: { file: '/work/plot.png', format: 'png' }, columns: 24, rows: 9 })
})

test('turning automatic previews off leaves tool rows alone', { options: { automatic: false } }, async ($, on) => {
  const fake = engine(on, { TERM_PROGRAM: 'ghostty' })

  await $.tool.call({ tool: 'Read', file_path: 'plot.png' })
  expect(fake.argv).toEqual([])
  expect(await $.ui.render(toolResult(fake.toolUseId))).toEqual(ENGINE_ROW)
})

test('image_preview shows the files it finds and tells the model', async ($, on) => {
  const fake = engine(on, { TERM_PROGRAM: 'herdr' })

  const ran = await $.tool.call({ tool: 'mcp__pixlens__image_preview', tool_use_id: 'toolu_preview', paths: ['plot.png', 'gone.png'] })
  expect(ran.result).toBe('Shown to the user:\nplot.png (640x480)\n1 path(s) not found')
  expect(fake.argv).toHaveLength(1)

  expect(orangeRows(await $.ui.render(toolResult('toolu_preview')))).toBe(15)
})

test('image_preview refuses when nothing is there', async ($, on) => {
  engine(on, { TERM_PROGRAM: 'herdr' })

  const ran = await $.tool.call({ tool: 'mcp__pixlens__image_preview', paths: ['gone.png'] })
  expect(ran.isError ?? ran.deny !== undefined).toBe(true)
})

test('/pixlens draws its images under its own output row', async ($, on) => {
  engine(on, { TERM_PROGRAM: 'herdr' })

  const ran = await $.command.run({
    command: 'pixlens',
    args: '"plot.png"',
    origin: { kind: 'composer' },
    presentation: { isFullscreen: false, columns: 100 },
  })
  expect(ran.text).toBe('plot.png (640x480)')

  const tree = await $.ui.render({
    surface: 'terminal',
    component: 'CommandOutput',
    requestId: 'msg_1',
    viewport: { columns: 100, rows: 40 },
    props: { command: 'pixlens', args: '"plot.png"', text: ran.text ?? '', isErrored: false },
  })
  expect(orangeRows(tree)).toBe(15)
})
