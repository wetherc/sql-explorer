/**
 * The topics of the guide inside the application.
 *
 * The application bundles no documentation of the repository, and the content
 * security policy allows no remote origin, so the text of the guide lives with
 * the code and travels with the build.
 *
 * Each topic is a Markdown file under `src/guide/`. The build reads the text
 * of the file, and the dialog turns it into HTML. No text of the user reaches
 * that HTML, so it needs no cleaning step.
 */
import { marked } from 'marked'
import startText from '@/guide/start.md?raw'

export interface GuideTopic {
  /** The name the list and the tests use. */
  id: string
  title: string
  /** The Markdown text of the topic. */
  body: string
}

export const GUIDE_TOPICS: GuideTopic[] = [
  { id: 'start', title: 'Where to start', body: startText },
]

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
