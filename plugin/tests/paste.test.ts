import { expect, test } from 'claude-code/testing'

import { band, DIR, ENGINE_ROW, find, session } from './fake'

test('pasted images show as thumbnails above the prompt, JPEG too', { options: { tool: false } }, async ($, on) => {
  const draft = { text: '' }
  const { argv, clock } = await session($, on, draft, ['1.png', '2.jpg'])
  expect(await $.ui.render(band())).toEqual(ENGINE_ROW)

  draft.text = 'why is [Image #1] wider than [Image #2]?'
  await clock.advance(250)

  expect(argv.map(a => a[2])).toEqual([`${DIR}/1.png`, `${DIR}/2.jpg`])
  expect(argv[0]).toEqual(['pixlens', 'render', `${DIR}/1.png`, '--columns', '20', '--rows', '3', '--cells', '--glyphs', 'quad'])

  const tree = await $.ui.render(band())
  expect(find(tree, 'Button').map(b => (b.props as { key?: string }).key)).toEqual(['paste-0', 'paste-1'])
  expect(JSON.stringify(tree)).toContain('#2')
  expect(JSON.stringify(tree)).toContain('"ref":0')
})

test('typing more words does not make thumbnails again; sending clears them', { options: { tool: false } }, async ($, on) => {
  const draft = { text: '[Image #1] ' }
  const { argv, clock } = await session($, on, draft, ['1.png'])
  await clock.advance(250)
  draft.text = '[Image #1] what is this'
  await clock.advance(1000)
  expect(argv).toHaveLength(1)

  draft.text = ''
  await clock.advance(250)
  expect(await $.ui.render(band())).toEqual(ENGINE_ROW)
})

test('a tag whose file is not there yet says so, then fills in', { options: { tool: false } }, async ($, on) => {
  const draft = { text: '[Image #3]' }
  const files: string[] = []
  const { argv, clock } = await session($, on, draft, files)
  await clock.advance(250)
  expect(JSON.stringify(await $.ui.render(band()))).toContain('no preview')

  files.push('3.png')
  await clock.advance(250)
  expect(argv).toHaveLength(1)
  expect(find(await $.ui.render(band()), 'Button')).toHaveLength(1)
})
