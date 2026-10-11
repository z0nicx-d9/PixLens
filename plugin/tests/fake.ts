import type { On, RenderElement } from 'claude-code'
import { mock } from 'claude-code/testing'
import type { Engine } from 'claude-code/testing'

export const ENGINE_ROW: RenderElement = { type: 'engine', ref: 0 }
export const DIR = '/tmp/claude-501/-Users-me-proj/sess-1/images'
/** The tag the fake CLI hands back for `render --overlay`. */
export const TAG = '\u28FF\u2801\u2802\u2803'

export type Draft = { text: string }

/** Upper half-blocks, orange over the default background. */
function cells(columns: number, rows: number): string {
  const words = Uint32Array.from({ length: columns * rows * 3 }, (_, i) => [0x2580, 0xff8800, 0x01000000][i % 3] ?? 0)
  return btoa(String.fromCharCode(...new Uint8Array(words.buffer)))
}

function entry(name: string, kind: 'file' | 'dir') {
  return { name, kind, size: 1, mtimeMs: 0, isLink: false }
}

/**
 * A session started in /Users/me/proj whose paste cache holds `files`, a
 * draft the test edits, and a CLI that fits every picture as 16 x 6 cells.
 */
export async function session($: Engine, on: On, draft: Draft, files: string[], hasOverlay = false) {
  const argv: string[][] = []
  const opened: string[] = []
  const toasts: string[] = []
  const clock = mock.clock(on, { now: 0 })
  mock.env(on, { TERM_PROGRAM: 'herdr' })

  on('session.start', ($, e) => ({ cwd: e.cwd }))
  on('command.register', () => ({ value: { command: 'pixlens' } }))
  on('session.id', () => ({ value: 'sess-1' }))
  on('session.root', () => ({ value: '/Users/me/proj' }))
  on('prompt.read', () => ({ value: { text: draft.text, cursor: draft.text.length } }))
  on('fs.list', ($, e) => {
    if (e.path === '/tmp') return { value: [entry('claude-501', 'dir'), entry('notes.txt', 'file')] }
    if (e.path === DIR) return { value: files.map(name => entry(name, 'file')) }
    return { value: [] }
  })
  on('fs.exists', ($, e) => ({ value: e.path === DIR }))
  on('process.run', ($, e) => {
    const [, command, file] = e.argv
    if (command === 'overlay') {
      const answer = hasOverlay ? { overlay: 'started', ready: true } : { error: 'Pixlens Overlay.app is not installed' }
      return { value: { exitCode: 0, stdout: JSON.stringify(answer), stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
    }
    argv.push([...e.argv])
    const tag = e.argv.includes('--overlay') ? { tag: TAG } : {}
    const answer =
      command === 'copy'
        ? { copied: file }
        : command === 'open'
          ? { opened: file }
          : command === 'menu'
            ? { menu: file }
          : { path: file, format: 'png', width: 800, height: 300, columns: 16, rows: 6, cells: cells(16, 6), ...tag }
    return { value: { exitCode: 0, stdout: JSON.stringify(answer), stderr: '', isStdoutTruncated: false, isStderrTruncated: false } }
  })
  on('ui.render', () => ENGINE_ROW)
  on('ui.open', ($, e) => {
    opened.push(e.id)
    return { value: { isPlaced: true } }
  })
  on('ui.toast', ($, e) => {
    toasts.push(JSON.stringify(e))
    return { value: undefined }
  })

  await $.session.start({ cwd: '/Users/me/proj', surface: 'terminal', isInteractive: true })
  return { argv, clock, opened, toasts }
}

export function band(bodyColumns = 95) {
  return {
    surface: 'terminal',
    component: 'AbovePrompt',
    requestId: 'band',
    viewport: { columns: bodyColumns + 5, rows: 40 },
    props: { hasSurvey: false, isWorking: false, maxRows: 40, bodyColumns, scroll: { offset: 0, bodyRows: 39 }, view: {} },
  } as const
}

export function find(tree: RenderElement | string, type: string): Extract<RenderElement, { props: unknown }>[] {
  if (typeof tree === 'string') return []
  const own = tree.type === type && 'props' in tree ? [tree as Extract<RenderElement, { props: unknown }>] : []
  const children = 'children' in tree && Array.isArray(tree.children) ? tree.children : []
  return [...own, ...children.flatMap(child => find(child as RenderElement | string, type))]
}
