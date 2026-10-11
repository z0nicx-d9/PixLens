import { expect, test } from 'claude-code/testing'

import { band, DIR, session, TAG } from './fake'

test('clicking a thumbnail shows the file full size', { options: { tool: false } }, async ($, on) => {
  const { argv, clock, toasts } = await session($, on, { text: 'look at [Image #1]' }, ['1.png'])
  await clock.advance(250)
  await $.ui.render(band())

  await $.ui.press({ plugin: 'pixlens', key: 'paste-0' })
  expect(argv.at(-1)?.slice(0, 4)).toEqual(['pixlens', 'menu', `${DIR}/1.png`, '--app'])
  // The overlay app the plugin ships.
  expect(argv.at(-1)?.[4]).toMatch(/\/overlay\/Pixlens Overlay\.app$/)
  expect(toasts).toEqual([])
})

test('/pixlens copy and open take a pasted #n', { options: { tool: false } }, async ($, on) => {
  const { argv } = await session($, on, { text: '' }, ['1.png', '2.jpg'])
  const run = (args: string) =>
    $.command.run({ command: 'pixlens', args, origin: { kind: 'composer' }, presentation: { isFullscreen: true, columns: 100 } })

  expect((await run('copy #2')).text).toBe('Copied #2: ctrl+v pastes it')
  expect(argv.at(-1)).toEqual(['pixlens', 'copy', `${DIR}/2.jpg`])

  expect((await run('open #1')).text).toBe('Opened #1')
  expect(argv.at(-1)).toEqual(['pixlens', 'open', `${DIR}/1.png`])

  expect((await run('open #9')).text).toBe('pixlens: no image #9')
  expect((await run('')).text).toContain('/pixlens open <image|#n>')
})

test('a block picture is plain text, never inside its Button', { options: { tool: false, mode: 'blocks' } }, async ($, on) => {
  const { clock } = await session($, on, { text: 'look at [Image #1]' }, ['1.png'])
  await clock.advance(250)
  const ui = await $.ui.mount({ plugin: 'pixlens', ...band() })

  const buttons = await ui.findAll({ type: 'Button' })
  expect(buttons.map(b => b.key)).toEqual(['paste-0'])
  expect(buttons[0]?.text.trim()).toBe('#1')
})

test('with Pixlens Overlay running, a block picture carries its tag first', { options: { tool: false } }, async ($, on) => {
  const { argv, clock } = await session($, on, { text: 'look at [Image #1]' }, ['1.png'], true)
  await clock.advance(250)
  const ui = await $.ui.mount({ plugin: 'pixlens', ...band() })

  expect(argv.at(-1)).toContain('--overlay')
  const texts = await ui.findAll({ type: 'Text' })
  expect(texts.find(t => t.text.includes(TAG))?.text.startsWith(TAG)).toBe(true)
  // The cells are blank under a ready overlay: no colors drawn.
  expect(texts.some(t => (t.props as { backgroundColor?: string }).backgroundColor !== undefined)).toBe(false)
  // The overlay handles the name, so no Button inverts under its ring.
  expect(await ui.findAll({ type: 'Button' })).toEqual([])
  expect(texts.some(t => t.text.trim() === '#1')).toBe(true)
})

test('without the overlay, no tag and no --overlay', { options: { tool: false } }, async ($, on) => {
  const { argv, clock } = await session($, on, { text: 'look at [Image #1]' }, ['1.png'])
  await clock.advance(250)
  const ui = await $.ui.mount({ plugin: 'pixlens', ...band() })

  expect(argv.at(-1)).not.toContain('--overlay')
  expect((await ui.findAll({ type: 'Text' })).some(t => t.text.includes(TAG))).toBe(false)
})
