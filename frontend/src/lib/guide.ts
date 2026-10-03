/**
 * The topics of the guide inside the application.
 *
 * The content security policy allows no remote origin, so the build bundles
 * the text of the guide. Each topic is a Markdown file under `docs/guide/` at
 * the root of the repository. The GitHub Pages site publishes the same files,
 * so the application and the site show one text.
 *
 * Each file starts with Jekyll front matter, which gives the title of the
 * topic and its place in the list. The dialog turns the rest of the file into
 * HTML. No text of the user reaches that HTML, so it needs no cleaning step.
 */
import { marked } from 'marked'

const FILES = import.meta.glob<string>('../../../docs/guide/*.md', {
  query: '?raw',
  import: 'default',
  eager: true,
})

export interface GuideTopic {
  /** The name the list and the tests use, which is the name of the file. */
  id: string
  title: string
  /** The place of the topic in the list, from the `order` field. */
  order: number
  /** The Markdown text of the topic, without its front matter. */
  body: string
}

const FRONT_MATTER = /^---\r?\n([\s\S]*?)\r?\n---\r?\n+/

/**
 * Reads one file of the guide. A file without front matter, a title or an
 * order throws, so the tests stop a file that the list cannot place.
 */
export function parseTopic(path: string, text: string): GuideTopic {
  const id = path.replace(/^.*\//, '').replace(/\.md$/, '')
  const match = FRONT_MATTER.exec(text)
  const fields = new Map<string, string>()
  for (const line of (match?.[1] ?? '').split(/\r?\n/)) {
    const colon = line.indexOf(':')
    if (colon > 0) fields.set(line.slice(0, colon).trim(), line.slice(colon + 1).trim())
  }
  const title = fields.get('title')
  const order = Number(fields.get('order'))
  if (!match || !title || !Number.isFinite(order)) {
    throw new Error(`The guide file '${id}' needs a title and an order in its front matter.`)
  }
  return { id, title, order, body: text.slice(match[0].length) }
}

export const GUIDE_TOPICS: GuideTopic[] = Object.entries(FILES)
  .map(([path, text]) => parseTopic(path, text))
  .sort((a, b) => a.order - b.order)

/** Finds one topic by its name, or the first topic for a name it does not hold. */
export function topicById(id: string): GuideTopic {
  return GUIDE_TOPICS.find((topic) => topic.id === id) ?? GUIDE_TOPICS[0]!
}

/**
 * Turns the text of a topic into HTML. The first line of each file is the
 * title of the topic, which the dialog draws above the text, so the renderer
 * leaves it out.
 */
export function renderTopic(topic: GuideTopic): string {
  const body = topic.body.replace(/^#\s.*(\r?\n)+/, '')
  return marked.parse(body, { async: false })
}
