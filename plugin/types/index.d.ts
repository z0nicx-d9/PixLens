/** One image drawn in a tile: under a tool result, a /pixlens row, or above the prompt. */
export type Preview = {
  /** Absolute path of the source image. */
  path: string
  /** What the tile's caption says: the file name, or `#n` for a pasted image. */
  name: string
  /** Source size in pixels; 0 when unknown. */
  width: number
  height: number
  /** The box it is drawn in, in terminal cells. */
  columns: number
  rows: number
  /** A PNG sized for the box, which the terminal reads (`Image`). */
  png?: string
  /** Block cells, packed as the CLI prints them, drawn as colored text. */
  cells?: string
  /** Pixlens Overlay's tag, drawn in the first cells of a block picture. */
  tag?: string
  /** Why it could not be drawn, shown in place of the picture. */
  error?: string
}

/** The previews of one /pixlens run, found again by its arguments. */
export type CommandPreviews = { args: string; previews: Preview[] }

declare module 'claude-code' {
  interface PluginState {
    pixlens: {
      /** Per tool call, by its tool_use_id. */
      previews: StateFamily<Preview[]>
      /** The last /pixlens runs, newest last. */
      commands: CommandPreviews[]
      /** The images pasted into the draft, in tag order. */
      pasted: Preview[]
    }
  }
}
