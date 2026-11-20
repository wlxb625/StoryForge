import { describe, expect, it } from 'vitest'
import type { Chapter, ModelProfile, Project, Revision, Scene } from '../contracts/types'
import { cjkIndex, cjkQuery } from '../db/fts'
import { freezeContext } from './freeze'
import { emptyAudience, emptyMemory, extractCandidates, memoryAnalysis, sourceAnchor, writerMemory, type Fact } from './memory'

const book: Project = { title: '渡口', projectId: 'novel', path: 'local.story', targetBeatChars: 900, targetChapterChars: 2000 }
const chapter: Chapter = { chapterId: 'next', chapterOrder: 2, currentRevisionId: 'r2', title: '船票', status: 'accepted' }
const model = { id: 'model', model: 'profile', version: '1', tokenLimitField: 'max_tokens', capabilities: {} } as ModelProfile
const secret = '周宁调包账本，真账本在七号锅炉夹层'
const revision: Revision = { revisionId: 'r2', parentRevisionId: null, createdAt: '', saveKind: 'formal_revision',
  content: `🗝${secret}。许眠走进渡口。`, sourceTag: 'manual_edit', cjkCharCount: 18 }
const scene: Scene = { sceneId: 'scene', title: secret, version: 1,
  details: { povCharacterId: 'hero', conflict: '锅炉旁还留着刚画的墨蛾印', time: '', location: '', cast: '' } }

function adversarialMemory() {
  const memory = emptyMemory()
  memory.canon = [
    { entryId: 'hero', version: 1, category: 'character', name: '许眠', aliases: [], body: '刚到渡口的记账员。', locked: true,
      audience: { ...emptyAudience(), scope: 'public' } },
    { entryId: 'secret', version: 2, category: 'world_rule', name: '藏本地点', aliases: ['墨蛾印'], body: secret, locked: true,
      audience: { ...emptyAudience(), protectedTerms: ['调包账本','七号锅炉夹层','墨蛾印'], observableClue: '账本边沿有一线白胶。' } },
  ]
  const fact: Fact = { eventId: 'swap', chapterId: 'prior', chapterOrder: 1, sourceRevisionId: 'r1', eventType: 'secret_created',
    approval: 'author_accepted', confidence: 'high', current: true, subjects: [], summary: secret, time: '',
    anchor: { quote: secret, startOffset: 0, endOffset: secret.length },
    audience: { ...emptyAudience(), observableClue: '账本封底比封面新。' } }
  memory.events = [fact, { ...fact, eventId: 'summary', eventType: 'chapter_summary' }]
  memory.notes = [{ noteId: 'note', sourceRevisionId: 'r1', chapterOrder: 1, noteType: 'foreshadow', relatedIds: ['swap'],
    body: '回收墨蛾印', audience: { ...emptyAudience(), scope: 'public' }, status: 'open' }]
  return memory
}

describe('证据与视角隔离', () => {
  it('排除未获知事实、摘要、备注、原文、场景和草稿，只提供可观察线索', () => {
    const capture = freezeContext(book, chapter, revision, model, '让许眠查看账本', secret, {
      scene, memory: adversarialMemory(), promptRevision: { promptRevisionId: 'unsafe', instruction: secret, createdAt: '' },
    })
    const request = JSON.stringify(capture.requestBody)
    for (const term of ['调包账本','七号锅炉夹层','墨蛾印','藏本地点','已确认正文末尾']) expect(request).not.toContain(term)
    expect(request).toContain('白胶')
    expect(request).toContain('封底比封面新')
    expect(request).toContain('许眠')
    expect(() => freezeContext(book, chapter, revision, model, '写出调、包 账本的经过', '', {
      scene, memory: adversarialMemory(),
    })).toThrow(/不可见/)
  })

  it('区分真知和误信，独立审查拿到全知数据', () => {
    const memory = adversarialMemory()
    memory.knowledge.push({ characterId: 'hero', eventId: 'swap', sinceRevisionId: 'r1', sinceOrder: 1,
      state: 'misbelieves', source: 'inferred', perception: '她误以为账本被雨淋坏。', observableClue: '' })
    expect(writerMemory(memory, chapter, 'hero').context).toContain('被雨淋坏')
    expect(writerMemory(memory, chapter, 'hero').context).not.toContain(secret)
    const audit = memoryAnalysis('consistency_check', book, chapter, revision, model, memory, 'hero')
    const extract = memoryAnalysis('memory_extract', book, chapter, revision, model, memory)
    expect(audit.prompt).toContain('作者全知数据')
    expect(audit.prompt).toContain('回收墨蛾印')
    expect(extract.prompt).not.toContain('回收墨蛾印')
    expect(extract.prompt).toContain(revision.content)
    memory.canon = memory.canon.filter(item => item.entryId !== 'secret')
    memory.events = memory.events.filter(item => item.eventId === 'swap')
    memory.knowledge[0]!.state = 'knows'
    expect(writerMemory(memory, chapter, 'hero').context).toContain(secret)
    expect(writerMemory(memory, chapter, undefined).context).not.toContain(secret)
  })

  it('锚点使用原文 UTF-16 偏移并拒绝改写或歧义引文', () => {
    expect(sourceAnchor(revision.content, '周宁')?.startOffset).toBe(2)
    expect(sourceAnchor('出门。出门。', '出门')).toBeUndefined()
    expect(sourceAnchor('出门。出门。', '出门', 2)?.startOffset).toBe(3)
    const output = extractCandidates(JSON.stringify({ events: [
      { quote: '许眠走进渡口', eventType: 'location_move', time: '', subjects: [], confidence: 'high', objectiveRoutine: true, summary: '进了渡口' },
      { quote: '虚构引文', eventType: 'location_move', time: '', subjects: [], confidence: 'high', objectiveRoutine: true, summary: '无证据' },
    ] }), revision.content)
    expect(output.candidates).toHaveLength(1)
    expect(output.problem).toContain('锚点')
  })
})

it('中文叠字和增补区编码保留，切词始终停在标点及非汉字处', () => {
  expect(cjkIndex('周、宁')).not.toContain(cjkQuery('周宁'))
  expect(cjkIndex('周宁、𠮷利')).toBe(cjkQuery('周宁') + ' gap ' + cjkQuery('𠮷利'))
  expect(cjkQuery('周，宁')).toBe('')
})
