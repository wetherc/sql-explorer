import { describe, expect, it } from 'vitest'
import { GUIDE_TOPICS, topicById } from '@/lib/guide'

describe('GUIDE_TOPICS', () => {
  it('holds at least one topic, with a name and a text for each', () => {
    expect(GUIDE_TOPICS.length).toBeGreaterThan(0)
    for (const topic of GUIDE_TOPICS) {
      expect(topic.id).not.toBe('')
      expect(topic.title).not.toBe('')
      expect(topic.body.length).toBeGreaterThan(0)
    }
  })

  it('gives each topic a name of its own', () => {
    const names = GUIDE_TOPICS.map((topic) => topic.id)
    expect(new Set(names).size).toBe(names.length)
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
