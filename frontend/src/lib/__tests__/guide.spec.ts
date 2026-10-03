import { describe, expect, it } from 'vitest'
import { GUIDE_TOPICS, parseTopic, renderTopic, topicById } from '@/lib/guide'

describe('GUIDE_TOPICS', () => {
  it('holds at least one topic, with a name and a text for each', () => {
    expect(GUIDE_TOPICS.length).toBeGreaterThan(0)
    for (const topic of GUIDE_TOPICS) {
      expect(topic.id).not.toBe('')
      expect(topic.title).not.toBe('')
      expect(topic.body.length).toBeGreaterThan(0)
    }
  })

  it('gives each topic the title that its own text carries', () => {
    for (const topic of GUIDE_TOPICS) {
      expect(topic.body.split('\n')[0]).toBe(`# ${topic.title}`)
    }
  })

  it('leaves the title out of the text that the reader sees', () => {
    for (const topic of GUIDE_TOPICS) {
      expect(renderTopic(topic)).not.toContain(`<h1>${topic.title}</h1>`)
    }
  })

  it('gives each topic a name of its own', () => {
    const names = GUIDE_TOPICS.map((topic) => topic.id)
    expect(new Set(names).size).toBe(names.length)
  })

  it('lists the topics in the order of their front matter, with each place once', () => {
    const places = GUIDE_TOPICS.map((topic) => topic.order)
    expect(places).toEqual([...places].sort((a, b) => a - b))
    expect(new Set(places).size).toBe(places.length)
    expect(GUIDE_TOPICS[0]!.id).toBe('start')
  })
})

describe('parseTopic', () => {
  it('reads the name, the title and the order, and leaves the front matter out of the text', () => {
    const topic = parseTopic(
      '../../../docs/guide/tabs.md',
      '---\ntitle: Tabs: and more\norder: 4\nno colon here\n---\n\n# Tabs: and more\n\nText.\n',
    )
    expect(topic).toEqual({
      id: 'tabs',
      title: 'Tabs: and more',
      order: 4,
      body: '# Tabs: and more\n\nText.\n',
    })
  })

  it('reads front matter with Windows line ends', () => {
    const topic = parseTopic('a.md', '---\r\ntitle: A\r\norder: 1\r\n---\r\n# A\r\n')
    expect(topic.title).toBe('A')
    expect(topic.body).toBe('# A\r\n')
  })

  it('throws for a file without front matter, a title or an order', () => {
    expect(() => parseTopic('a.md', '# A\n')).toThrow("'a'")
    expect(() => parseTopic('a.md', '---\norder: 1\n---\n# A\n')).toThrow()
    expect(() => parseTopic('a.md', '---\ntitle: A\n---\n# A\n')).toThrow()
    expect(() => parseTopic('a.md', '---\ntitle: A\norder: first\n---\n# A\n')).toThrow()
  })
})

describe('renderTopic', () => {
  it('turns the text of a topic into HTML', () => {
    const html = renderTopic({
      id: 'x',
      title: 'A topic',
      order: 1,
      body: '# A topic\n\nOne **word** stands out.\n\n- first\n- second\n',
    })

    expect(html).toContain('<strong>word</strong>')
    expect(html).toContain('<li>first</li>')
    // The dialog draws the title above the text, so the text holds no copy.
    expect(html).not.toContain('<h1>')
  })

  it('keeps a text that starts with something other than a title', () => {
    const html = renderTopic({ id: 'x', title: 'A topic', order: 1, body: 'Plain words.\n' })
    expect(html).toContain('<p>Plain words.</p>')
  })

  it('renders each topic of the guide', () => {
    for (const topic of GUIDE_TOPICS) {
      expect(renderTopic(topic).length).toBeGreaterThan(0)
    }
  })
})

describe('topicById', () => {
  it('finds the topic of a name', () => {
    const first = GUIDE_TOPICS[0]!
    expect(topicById(first.id)).toBe(first)
  })

  it('falls back on the first topic for a name it does not hold', () => {
    expect(topicById('nowhere')).toBe(GUIDE_TOPICS[0])
  })
})
