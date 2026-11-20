import { expect, it } from 'vitest'
import { arcDirection, characterDiff, changeWindow, makeSpan, polishContext, suggestedEdit } from './editing'
import type { Chapter, ModelProfile, Project, Revision } from '../contracts/types'

it('keeps UTF-16 positions and exactly 500 surrounding characters in the edit prompt', () => {
  const before = '远'.repeat(150) + '近'.repeat(500)
  const after = '凳'.repeat(500) + '船'.repeat(140)
  const revision = { revisionId: 'r', content: before + '旧🗝' + after, contentHash: 'sha256' } as Revision
  const chapter = { chapterId: 'c', chapterOrder: 5, currentRevisionId: 'r' } as Chapter
  const span = makeSpan(revision, before.length, before.length + '旧🗝'.length)
  const profile = { capabilities: {}, tokenLimitField: 'max_tokens' } as ModelProfile
  const context = polishContext({} as Project, chapter, revision, profile, span, '')
  expect(context.span.expected).toBe('旧🗝')
  expect(context.prompt).toContain('近'.repeat(500))
  expect(context.prompt).toContain('凳'.repeat(500))
  expect(context.prompt).not.toMatch(/[远船]/)
})

it('produces reversible character-level red-green segments', () => {
  const left = '铜匙落地🗝，门开了。'
  const right = '铜匙晃了晃🔒，门没开。'
  const diff = characterDiff(left, right)
  expect(diff.filter(part => part.type !== 'added').map(part => part.text).join('')).toBe(left)
  expect(diff.filter(part => part.type !== 'removed').map(part => part.text).join('')).toBe(right)
  expect(suggestedEdit('下雨。', '下雨！')).toBe('surface')
  expect(changeWindow('铜匙留在北门', '铜匙转给周宁')).toContain('铜匙')
})

it('only uses pivot guidance in the chosen transition chapters', () => {
  const arc = { arcId: 'a',title:'第一段',roughGoal:'离开渡口',expectedChapters:8,milestones:[],status:'active' as const,
    pivotInstruction:'船家带路',pivotRemaining:2,pivotStartOrder:6,version:2,writerVisible:true }
  const chapter = { chapterOrder: 7 } as Chapter
  expect(arcDirection(arc, chapter)).toContain('第 2/2 章')
  expect(arcDirection(arc, { ...chapter, chapterOrder: 8 })).not.toContain('船家带路')
  expect(arcDirection({ ...arc, writerVisible: false }, chapter)).toBe('')
})
