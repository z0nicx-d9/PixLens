import { describe, expect, test } from 'claude-code/testing'

import { cellRuns, centered, fitCells, imagePaths, imageTags, isImagePath, pastedFile, pngSize, projectSlug, shrinkTo, splitArgs, tileLimit, withTag } from '../hooks/lib'

describe('imagePaths', () => {
  test('finds paths in a command and its output, once each, in order', () => {
    const text = 'python plot.py --out=charts/a.png && cp charts/a.png "b c/B.JPG"\nSaved ./out/final.webp.'
    expect(imagePaths(text)).toEqual(['charts/a.png', 'c/B.JPG', './out/final.webp'])
  })

  test('leaves URLs and non-images out', () => {
    expect(imagePaths('curl -o x.png https://example.com/x.png; cat notes.md')).toEqual(['x.png'])
  })

  test('stops at the limit', () => {
    expect(imagePaths('1.png 2.png 3.png', 2)).toEqual(['1.png', '2.png'])
  })
})

test('isImagePath knows the CLI formats, any case', () => {
  expect(isImagePath('/a/b.SVG')).toBe(true)
  expect(isImagePath('photo.jpeg')).toBe(true)
  expect(isImagePath('notes.txt')).toBe(false)
  expect(isImagePath('png')).toBe(false)
})

test('splitArgs keeps quoted paths whole', () => {
  expect(splitArgs(`a.png "my pics/b.jpg" 'c d.gif'`)).toEqual(['a.png', 'my pics/b.jpg', 'c d.gif'])
  expect(splitArgs('   ')).toEqual([])
})

describe('fitCells matches the CLI', () => {
  test('wide image is bounded by columns', () => {
    expect(fitCells(1920, 1080, 60, 40)).toEqual({ columns: 60, rows: 17 })
  })
  test('tall image is bounded by rows', () => {
    expect(fitCells(1000, 3000, 80, 20)).toEqual({ columns: 13, rows: 20 })
  })
  test('small image is not scaled up', () => {
    expect(fitCells(16, 16, 80, 24)).toEqual({ columns: 16, rows: 8 })
  })
  test('degenerate sizes give one cell', () => {
    expect(fitCells(0, 0, 0, 0)).toEqual({ columns: 1, rows: 1 })
  })
})

test('shrinkTo keeps the aspect ratio', () => {
  expect(shrinkTo({ columns: 60, rows: 17 }, 30)).toEqual({ columns: 30, rows: 9 })
  expect(shrinkTo({ columns: 20, rows: 10 }, 30)).toEqual({ columns: 20, rows: 10 })
})

test('pngSize reads IHDR', () => {
  // The first 24 bytes of a 640x480 PNG: signature, IHDR length and type, width, height.
  const head = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, 0x49, 0x48, 0x44, 0x52, 0, 0, 2, 0x80, 0, 0, 1, 0xe0]
  expect(pngSize(btoa(String.fromCharCode(...head)))).toEqual({ width: 640, height: 480 })
  expect(pngSize(btoa('GIF89a this is not a png at all'))).toBeUndefined()
})

test('imageTags reads each pasted tag once, in order', () => {
  expect(imageTags('a [Image #2] b [Image #1] c [Image #2] [Image #x]')).toEqual([2, 1])
  expect(imageTags('no images')).toEqual([])
})

test('projectSlug spells the folder as Claude Code does', () => {
  expect(projectSlug('/Users/me/Desktop/Code/image_view')).toBe('-Users-me-Desktop-Code-image-view')
})

test('pastedFile finds the tag by number, any image extension', () => {
  expect(pastedFile(['10.png', '1.jpg', '1.txt'], 1)).toBe('1.jpg')
  expect(pastedFile(['10.png'], 1)).toBeUndefined()
})

test('tileLimit shares the row between tiles', () => {
  expect(tileLimit(1, 100, 40, 4)).toEqual({ columns: 20, rows: 4 })
  expect(tileLimit(3, 100, 40, 8, 60)).toEqual({ columns: 32, rows: 8 })
  expect(tileLimit(30, 100, 5, 8)).toEqual({ columns: 4, rows: 4 })
})

test('cellRuns groups a row by its colors and leaves the terminal default out', () => {
  // Three cells: two orange half-blocks, then a space on blue.
  const words = [0x2580, 0xff8800, 0x01000000, 0x2580, 0xff8800, 0x01000000, 0x20, 0x01000000, 0x0000ff]
  const bytes = new Uint8Array(Uint32Array.from(words).buffer)
  expect(cellRuns(btoa(String.fromCharCode(...bytes)), 3, 1)).toEqual([
    [
      { text: '▀▀', color: '#ff8800' },
      { text: ' ', backgroundColor: '#0000ff' },
    ],
  ])
})

test('centered cuts and centers a caption', () => {
  expect(centered('#1', 6)).toBe('  #1')
  expect(centered('a-long-name.png', 8)).toBe('a-long-…')
})

describe('withTag', () => {
  test('puts the tag in the first cells, the bottom tag in the last row, row marks with the id between', () => {
    const rows = [
      [{ text: 'ab', color: '#111111' }, { text: 'cdef', color: '#222222' }],
      [{ text: 'mnopqr' }],
      [{ text: 'ghijkl' }],
    ]
    expect(withTag(rows, '\u28FFxyz')).toEqual([
      [{ text: '\u28FFxyz', isDim: true }, { text: 'ef', color: '#222222' }],
      [{ text: '\u28FDxyz', isDim: true }, { text: 'qr' }],
      [{ text: '\u28FExyz', isDim: true }, { text: 'kl' }],
    ])
  })

  test('a one-row picture carries the top tag alone', () => {
    expect(withTag([[{ text: 'abcdef' }]], '\u28FFxyz')).toEqual([[{ text: '\u28FFxyz', isDim: true }, { text: 'ef' }]])
  })

  test('leaves the rows alone without a tag', () => {
    const rows = [[{ text: 'ab' }]]
    expect(withTag(rows, undefined)).toBe(rows)
  })
})
