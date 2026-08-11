import { describe, expect, it } from 'vitest'
import { GUIDE_TOPICS, renderTopic, topicById } from '@/lib/guide'

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
})

describe('renderTopic', () => {
  it('turns the text of a topic into HTML', () => {
    const html = renderTopic({
      id: 'x',
      title: 'A topic',
      body: '# A topic\n\nOne **word** stands out.\n\n- first\n- second\n',
    })

    expect(html).toContain('<strong>word</strong>')
    expect(html).toContain('<li>first</li>')
    // The dialog draws the title above the text, so the text holds no copy.
    expect(html).not.toContain('<h1>')
  })

  it('keeps a text that starts with something other than a title', () => {
    const html = renderTopic({ id: 'x', title: 'A topic', body: 'Plain words.\n' })
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
