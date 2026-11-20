import type { Chapter, ModelProfile, Project, Revision } from '../contracts/types'
import { analysisContext } from './memory'

export type EditLevel = 'surface' | 'semantic' | 'canon'
export interface EditSpan { start: number; end: number; expected: string; contentHash: string }
export interface Milestone { id: string; text: string; done: boolean }
export interface StageArc {
  arcId: string; title: string; roughGoal: string; expectedChapters: number; milestones: Milestone[]
  pivotInstruction: string; pivotRemaining: number; pivotStartOrder: number; status: 'active' | 'completed'
  version: number; writerVisible: boolean
}
export interface ImpactNote { noteId: string; body: string; noteType: string; chapterId: string }
export interface PivotPreview { version: number; suggested: number; previousGoal: string; notes: ImpactNote[] }
export interface Pivot { arcId: string; version: number; nextGoal: string; instruction: string; transitionChapters: number; noteIds: string[] }
export interface DiffSegment { type: 'same' | 'removed' | 'added'; text: string }

export function makeSpan(revision: Revision, start: number, end: number): EditSpan {
  const contentHash = revision.contentHash
  if (!contentHash || !Number.isInteger(start) || !Number.isInteger(end) || start < 0 || end <= start
    || end > revision.content.length || end - start > 1800) throw new Error('请选择当前正式修订中 1 至 1800 字的连续文字。')
  return { start, end, expected: revision.content.slice(start, end), contentHash }
}

export function polishContext(project: Project, chapter: Chapter, revision: Revision, profile: ModelProfile,
  span: EditSpan, instruction: string) {
  const before = Array.from(revision.content.slice(0, span.start)).slice(-500).join('')
  const after = Array.from(revision.content.slice(span.end)).slice(0, 500).join('')
  const prompt = [
    '你是中文小说编辑。仅精修选区，保留人物视角、事件结果、人物掌握的信息和事实。只输出一个 JSON 对象，字段 replacement 是替换文字。',
    '作者要求：' + (instruction.trim() || '收紧语言，保留语气与信息量。'),
    '前文：\n' + before, '选区：\n' + span.expected, '后文：\n' + after,
  ].join('\n\n')
  return { ...analysisContext(project, chapter, revision, profile, prompt), span }
}

export function criticContext(project: Project, chapter: Chapter, revision: Revision, profile: ModelProfile) {
  return analysisContext(project, chapter, revision, profile, [
    '你是中文小说主编。评估人物能动性、表层行为和潜台词、节奏、对话区分度、情感积累及套话。只给有原文证据的建议，保留作者选择和文字。',
    '仅返回 JSON {"findings":[]}。每项为 severity(severe/notice)、message、suggestion、quote、occurrence。quote 为逐字原文，最长 120 字。重复时写第几次 occurrence，从 1 起。',
    '正文：\n' + revision.content,
  ].join('\n\n'))
}

export function parseReplacement(output: string) {
  const payload: unknown = JSON.parse(output.trim().replace(/^```(?:json)?\s*/i, '').replace(/\s*```$/, ''))
  const text = (payload as { replacement?: unknown } | null)?.replacement
  if (typeof text !== 'string' || text.length > 3600) throw new Error('建议缺少可采纳的替换文字。')
  return text
}

export function characterDiff(original: string, next: string): DiffSegment[] {
  const left = Array.from(original)
  const right = Array.from(next)
  const stride = right.length + 1
  const lengths = new Uint16Array((left.length + 1) * stride)
  for (let a = left.length - 1; a >= 0; a--) {
    for (let b = right.length - 1; b >= 0; b--) {
      const index = a * stride + b
      lengths[index] = left[a] === right[b] ? lengths[index + stride + 1]! + 1
        : Math.max(lengths[index + stride]!, lengths[index + 1]!)
    }
  }
  const diff: DiffSegment[] = []
  function add(type: DiffSegment['type'], char: string) {
    const last = diff.at(-1)
    if (last?.type === type) last.text += char
    else diff.push({ type, text: char })
  }
  let a = 0
  let b = 0
  while (a < left.length || b < right.length) {
    if (a < left.length && b < right.length && left[a] === right[b]) {
      add('same', left[a++]!)
    } else if (b === right.length || (a < left.length && lengths[(a + 1) * stride + b]! >= lengths[a * stride + b + 1]!)) {
      add('removed', left[a++]!)
      continue
    } else {
      add('added', right[b++]!)
      continue
    }
    b++
  }
  return diff
}

export function changeWindow(before: string, after: string) {
  let start = 0
  while (start < Math.min(before.length, after.length) && before[start] === after[start]) start++
  let suffix = 0
  while (suffix < Math.min(before.length, after.length) - start && before.at(-suffix - 1) === after.at(-suffix - 1)) suffix++
  return before === after ? '' : [before, after].map(content => content.slice(Math.max(0, start - 36), content.length - suffix + 36)).join('\n')
}

export function suggestedEdit(before: string, after: string): EditLevel {
  const surface = (value: string) => value.replace(/[\p{P}\p{Z}\s]/gu, '')
  return before.trim() && surface(before) === surface(after) ? 'surface' : 'semantic'
}

export function arcDirection(arc: StageArc | undefined, chapter: Chapter) {
  if (!arc?.writerVisible || arc.status !== 'active') return ''
  const at = chapter.chapterOrder - arc.pivotStartOrder
  const transition = at >= 0 && at < arc.pivotRemaining
  const steps = arc.milestones.filter(step => !step.done).slice(0, 3).map(step => step.text)
  return [`阶段「${arc.title}」目标：${arc.roughGoal}`, steps.length ? '下一批路标：' + steps.join(' / ') : '',
    transition && arc.pivotInstruction ? `改纲过渡第 ${at + 1}/${arc.pivotRemaining} 章：${arc.pivotInstruction}` : '',
  ].filter(Boolean).join('\n')
}
