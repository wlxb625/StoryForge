import { expect, it } from 'vitest'
import { builtins } from './builtins'
import { parseCartridge, parseSkill, serializeSkill } from './pack'
import { activeStyles, deriveIntent } from '../narrative/intent'
import { freezeContext, retarget } from '../narrative/freeze'
import type { Chapter, ModelProfile, Project, Scene, StyleCatalog } from '../contracts/types'

const city = builtins.find(pack => pack.itemId === 'storyforge.style.city')!
const noir = builtins.find(pack => pack.itemId === 'storyforge.style.noir')!
const studio: StyleCatalog = { packs: builtins, activePackId: null, activeBaseSkillId: null }

it('ships four valid base styles and eight valid techniques with examples', () => {
  expect(builtins.filter(pack => pack.slot === 'base_style')).toHaveLength(4)
  expect(builtins.filter(pack => pack.slot === 'technique')).toHaveLength(8)
  for (const pack of builtins.filter(pack => pack.itemKind === 'writing_skill')) {
    expect(parseSkill(serializeSkill(pack))).toMatchObject({ itemId: pack.itemId, version: pack.version, slot: pack.slot })
  }
  expect(() => parseSkill(serializeSkill(city).replace('priority: 40', 'priority: 400'))).toThrow()
  expect(() => parseSkill(serializeSkill(city) + '\n<script>')).toThrow()
  expect(parseCartridge(JSON.stringify({ id: 'genre.test', version: '1.0.0', name: '测试卡带',
    description: '让证据改变选择', stages: ['提出疑点', '核对原件'] })).metadata.stages).toHaveLength(2)
})

it('keeps one base and at most two techniques and defers humor to the emotional axis', () => {
  const scene: Scene = { sceneId: 'scene', version: 1, title: '葬礼谈判',
    details: { time: '', location: '', cast: '', conflict: '极度悲痛，并不想说话',
      intent: { humor: '允许' }, techniqueIds: ['storyforge.humor.callback', 'storyforge.emotion.objects', 'storyforge.dialogue.subtext'] } }
  const intent = deriveIntent(scene, '悲伤的送别')
  expect(intent.humor).toBe('禁止')
  expect(activeStyles(studio, scene, intent, '谈判').skills.map(pack => pack.metadata.axis)).not.toContain('humor')
  expect(activeStyles(studio, scene, intent, '谈判').skills.filter(pack => pack.slot === 'technique').length).toBeLessThanOrEqual(2)
})

it('freezes style revisions and uses the identical scene prompt for both model slots', () => {
  const project: Project = { projectId: 'p', title: '临时小说', path: 'book.story', targetBeatChars: 900, targetChapterChars: 2000 }
  const chapter: Chapter = { chapterId: 'c', title: '分账', chapterOrder: 1, currentRevisionId: null, status: 'planned' }
  const model: ModelProfile = { id: 'a', version: 'v1', name: '主笔', model: 'a', baseUrl: 'http://localhost/v1',
    needsKey: false, hasKey: false, direct: true, tokenLimitField: 'max_tokens', capabilities: {} }
  const cityContext = freezeContext(project, chapter, undefined, model, '在巷口查一张收据', '',
    { catalog: { ...studio, activeBaseSkillId: city.itemId } })
  const noirContext = freezeContext(project, chapter, undefined, model, '在巷口查一张收据', '',
    { catalog: { ...studio, activeBaseSkillId: noir.itemId } })
  expect(cityContext.prompt).toContain('当下街巷')
  expect(noirContext.prompt).toContain('先报告物证')
  expect(cityContext.prompt).not.toContain('先报告物证')
  expect(cityContext.skillVersions[city.itemId]).toBe('1.0.0')
  const other = retarget(cityContext, { ...model, id: 'b', model: 'b' })
  expect(other.prompt).toBe(cityContext.prompt)
  expect(other.requestBody.model).toBe('b')
  expect(other.skillVersions).toEqual(cityContext.skillVersions)
})
