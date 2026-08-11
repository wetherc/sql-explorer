/**
 * The topics of the guide inside the application.
 *
 * The application bundles no documentation of the repository, and the content
 * security policy allows no remote origin, so the text of the guide lives with
 * the code and travels with the build.
 */
export interface GuideTopic {
  /** The name the list and the tests use. */
  id: string
  title: string
  body: string
}

export const GUIDE_TOPICS: GuideTopic[] = [
  {
    id: 'start',
    title: 'Where to start',
    body:
      'Open a connection in the connections panel. The explorer then shows the ' +
      'objects of that connection. Open a tab, choose the connection at the top ' +
      'of the tab, and write a statement. The Run button runs the statement ' +
      'under the cursor.',
  },
]

/** Finds one topic by its name, or the first topic for a name it does not hold. */
export function topicById(id: string): GuideTopic {
  return GUIDE_TOPICS.find((topic) => topic.id === id) ?? GUIDE_TOPICS[0]!
}
