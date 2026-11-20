import type { Chapter, FrozenContext, ModelProfile, Project, Revision } from '../contracts/types'
import type { SceneReview, TextRange, ViewpointRange } from './scenes'

export interface Audience {
  scope: 'public' | 'named_characters' | 'narrator_only'
  visibleTo: string[]
  observableClue: string
  protectedTerms: string[]
}
export interface Canon {
  entryId: string
  version: number
  category: 'character' | 'prop' | 'location' | 'world_rule'
  name: string
  aliases: string[]
  body: string
  audience: Audience
  locked: boolean
}
export interface Anchor { quote: string; startOffset: number; endOffset: number }
export const eventTypes = {
  chapter_summary: '章节摘要', location_move: '地点移动', minor_extra: '次要人物',
  prop_transfer: '道具交接', secret_created: '秘密形成', secret_revealed: '秘密揭示',
  relation_shift: '关系改变', death: '死亡', irreversible_change: '不可逆变化', plot_event: '情节事件',
}
export interface Candidate {
  eventType: keyof typeof eventTypes
  subjects: string[]
  summary: string
  time: string
  anchor: Anchor
  confidence: 'high' | 'medium' | 'low'
  objectiveRoutine: boolean
}
export interface Fact extends Omit<Candidate, 'objectiveRoutine'> {
  eventId: string
  sourceRevisionId: string
  activeRevisionId?: string | null
  chapterId: string
  chapterOrder: number
  approval: 'pending' | 'auto_accepted' | 'author_accepted' | 'rejected'
  audience: Audience
  current: boolean
}
export interface Knowledge {
  characterId: string
  eventId: string
  state: 'knows' | 'suspects' | 'misbelieves'
  source: 'observed' | 'told' | 'inferred'
  sinceRevisionId: string
  sinceOrder?: number
  perception: string
  observableClue: string
}
export interface Note {
  noteId: string
  sourceRevisionId: string
  chapterOrder?: number
  noteType: 'foreshadow' | 'gag_hook' | 'subtext_cue'
  relatedIds: string[]
  body: string
  audience: Audience
  status: 'open' | 'paid_off' | 'reminded' | 'deprecated'
  remindAfterChapters?: number | null
}
export interface MemoryRoles { auditProfileId: string | null; extractionProfileId: string | null }
export interface Memory { canon: Canon[]; events: Fact[]; knowledge: Knowledge[]; notes: Note[]; roles: MemoryRoles; ingested: Array<[string, string]>; stamp: string }
export interface SearchHit { chapterId: string; revisionId: string; chapterOrder: number; title: string; excerpt: string }
export interface CanonImpact { hits: SearchHit[]; facts: Fact[]; notes: Note[] }
export const emptyAudience = (): Audience => ({ scope: 'narrator_only', visibleTo: [], observableClue: '', protectedTerms: [] })
export const emptyMemory = (): Memory => ({
  canon: [], events: [], knowledge: [], notes: [], ingested: [], roles: { auditProfileId: null, extractionProfileId: null }, stamp: '',
})
export function noteIsDue(note: Note, chapterOrder: number) {
  return note.status === 'open' && note.chapterOrder != null && !!note.remindAfterChapters
    && chapterOrder >= note.chapterOrder + note.remindAfterChapters
}
const isApproved = (fact: Fact) => fact.approval === 'auto_accepted' || fact.approval === 'author_accepted'

function normalized(value: string) {
  return value.normalize('NFKC').toLocaleLowerCase('zh-CN').replace(/[\p{P}\p{Z}\p{C}\s]/gu, '')
}

export function writerMemory(memory: Memory, chapter: Chapter, requestedPov?: string, matches: string[] = []) {
  const pov = memory.canon.find(item => item.category === 'character' && item.entryId === requestedPov)?.entryId
  const allows = (audience: Audience) => audience.scope === 'public'
    || (audience.scope === 'named_characters' && !!pov && audience.visibleTo.includes(pov))
  const perspectives = new Map(memory.knowledge.filter(k => k.characterId === pov
    && (k.sinceOrder ?? Infinity) <= chapter.chapterOrder && ((k.sinceOrder ?? Infinity) < chapter.chapterOrder
      || k.sinceRevisionId === chapter.currentRevisionId)).map(k => [k.eventId, k]))
  const eventVisible = (fact: Fact) => {
    const perspective = perspectives.get(fact.eventId)
    return fact.current && isApproved(fact) && fact.chapterOrder <= chapter.chapterOrder
      && (perspective ? perspective.state === 'knows' : allows(fact.audience))
  }
  const restrictedCanon = memory.canon.filter(item => !allows(item.audience))
  const restrictedEvents = memory.events.filter(item => item.approval !== 'rejected' && !eventVisible(item))
  const visibleIds = new Set([...memory.canon.filter(item => allows(item.audience)).map(item => item.entryId),
    ...memory.events.filter(eventVisible).map(item => item.eventId)])
  const noteVisible = (note: Note) => (note.chapterOrder ?? Infinity) <= chapter.chapterOrder
    && allows(note.audience) && note.relatedIds.every(id => visibleIds.has(id))
  const restrictedNotes = memory.notes.filter(item => item.status !== 'deprecated' && !noteVisible(item))
  const terms = new Set([
    ...restrictedCanon.flatMap(item => [item.name, ...item.aliases, item.body, ...item.audience.protectedTerms]),
    ...restrictedEvents.flatMap(item => [item.summary, item.anchor.quote, ...item.audience.protectedTerms]),
    ...restrictedNotes.flatMap(item => [item.body, ...item.audience.protectedTerms]),
  ].map(normalized).filter(term => term.length >= 2))
  const safe = (value: string) => {
    const candidate = normalized(value)
    return [...terms].some(term => candidate.includes(term)) ? '' : value
  }
  const canon = memory.canon.filter(item => allows(item.audience)).sort((a, b) => Number(b.locked) - Number(a.locked))
  const matchedChapters = new Set(matches)
  const visibleFacts = memory.events.filter(eventVisible)
  const immediate = visibleFacts.slice(-20)
  const facts = [...new Map([...visibleFacts.filter(fact => matchedChapters.has(fact.chapterId)).slice(-10), ...immediate]
    .map(fact => [fact.eventId, fact])).values()]
  const perceptions = memory.events.filter(fact => fact.current && isApproved(fact) && fact.chapterOrder <= chapter.chapterOrder)
    .flatMap(fact => {
      const perspective = perspectives.get(fact.eventId)
      if (perspective?.state === 'suspects' || perspective?.state === 'misbelieves') {
        const perception = safe(perspective.perception)
        return perception ? [(perspective.state === 'suspects' ? '人物怀疑：' : '人物误信：') + perception] : []
      }
      return []
    })
  const clues = [...restrictedCanon.map(item => item.audience.observableClue),
    ...restrictedEvents.filter(fact => fact.current && isApproved(fact) && fact.chapterOrder <= chapter.chapterOrder)
      .map(fact => perspectives.get(fact.eventId)?.observableClue || fact.audience.observableClue),
    ...restrictedNotes.filter(note => (note.chapterOrder ?? Infinity) <= chapter.chapterOrder).map(note => note.audience.observableClue),
  ].map(safe).filter(Boolean)
  const factLines = facts.map(item => ({ item, line: safe(`（第 ${item.chapterOrder} 章）${item.eventType === 'chapter_summary' ? '可用摘要' : '已确认事实'}：${item.summary}`) }))
  const lines = [
    ...canon.map(item => safe(`${item.locked ? '作者锁定设定' : '作者设定'} · ${item.name}：${item.body}`)),
    ...factLines.map(({ line }) => line),
    ...memory.notes.filter(note => noteVisible(note) && ['open','reminded'].includes(note.status)).slice(-10)
      .map(note => safe(`${noteIsDue(note, chapter.chapterOrder) ? '到期回收提醒（可延后）' : '可用叙事备注'}：${note.body}`)),
    ...perceptions.slice(-10), ...[...new Set(clues)].slice(-15).map(clue => '可观察：' + clue),
  ].filter(Boolean)
  const included = new Set<string>()
  let used = 0
  for (const line of lines) {
    if (used + line.length >= 12000) break
    included.add(line)
    used += line.length + 1
  }
  return {
    guarded: restrictedCanon.length + restrictedEvents.length + restrictedNotes.length > 0,
    safe, povName: safe(canon.find(item => item.entryId === pov)?.name ?? ''),
    context: [...included].join('\n'),
    evidence: factLines.filter(({ line }) => included.has(line)).map(({ item }) => ({
      eventId: item.eventId, chapterId: item.chapterId, chapterOrder: item.chapterOrder,
      revisionId: item.activeRevisionId ?? item.sourceRevisionId, anchor: item.anchor, summary: item.summary,
    })),
    versions: Object.fromEntries(canon.map(item => [item.entryId, item.version])),
    searchableNames: canon.flatMap(item => [item.name, ...item.aliases]).map(safe).filter(Boolean),
    protectedTerms: [...new Set([...restrictedCanon, ...restrictedEvents, ...restrictedNotes]
      .flatMap(item => item.audience.protectedTerms).filter(term => term.length >= 2))],
  }
}

function protectedTermFindings(memory: Memory, chapter: Chapter, content: string, viewpoints: ViewpointRange[]) {
  const findings: NonNullable<FrozenContext['viewpointAlerts']> = []
  const forbidden = new Map<string | null, string[]>()
  for (const range of viewpoints) {
    if (!forbidden.has(range.povCharacterId)) {
      forbidden.set(range.povCharacterId, writerMemory(memory, chapter, range.povCharacterId ?? undefined).protectedTerms)
    }
    for (const term of forbidden.get(range.povCharacterId) ?? []) {
      const position = content.indexOf(term, range.start)
      if (position < 0 || position + term.length > range.end) continue
      findings.push({ severity: 'severe', message: '正文出现当前视角无权获知的受保护词。',
        suggestion: '使用此人物能观察到的线索，或核实本段视角和知情记录。',
        anchor: { quote: term, startOffset: position, endOffset: position + term.length } })
      if (findings.length >= 30) return findings
    }
  }
  return findings
}

export function sourceAnchor(content: string, quote: unknown, occurrence?: unknown, ranges?: TextRange[]): Anchor | undefined {
  if (typeof quote !== 'string' || !quote.trim() || quote.length > 240) return
  if (occurrence !== undefined && (typeof occurrence !== 'number' || !Number.isInteger(occurrence) || occurrence < 1 || occurrence > 100)) return
  let at = content.indexOf(quote)
  let match: Anchor | undefined
  let count = 0
  while (at >= 0) {
    const end = at + quote.length
    if (!ranges || ranges.some(range => range.start <= at && range.end >= end)) {
      if (occurrence === undefined && match) return
      match = { quote, startOffset: at, endOffset: end }
      if (++count === occurrence) return match
    }
    at = content.indexOf(quote, at + 1)
  }
  if (occurrence === undefined) return match
}

function object(value: unknown): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value)
}
function jsonList(text: string, field: string): unknown[] {
  const raw = text.trim().replace(/^```(?:json)?\s*/i, '').replace(/\s*```$/, '')
  const parsed: unknown = JSON.parse(raw)
  if (!object(parsed) || !Array.isArray(parsed[field]) || parsed[field].length > 30) throw new Error('模型返回的清单格式有误。')
  return parsed[field]
}

export function extractCandidates(text: string, content: string, ranges?: TextRange[]) {
  const candidates: Candidate[] = []
  const values = jsonList(text, 'events')
  for (const value of values) {
    if (!object(value)) continue
    const { eventType, subjects, summary, time, confidence, objectiveRoutine } = value
    const anchor = sourceAnchor(content, value.quote, value.occurrence, ranges)
    if (typeof eventType !== 'string' || !Object.hasOwn(eventTypes, eventType) || !Array.isArray(subjects)
      || subjects.some(id => typeof id !== 'string') || subjects.length > 15 || !anchor
      || typeof summary !== 'string' || !summary.trim() || summary.length > 750
      || typeof time !== 'string' || time.length > 60
      || (confidence !== 'high' && confidence !== 'medium' && confidence !== 'low')) continue
    candidates.push({ eventType: eventType as Candidate['eventType'], subjects, summary, time, anchor, confidence,
      objectiveRoutine: objectiveRoutine === true })
  }
  return { candidates, problem: candidates.length === values.length ? '' : '部分候选缺少可核验的原文锚点或字段。' }
}

export interface Finding { severity: 'severe' | 'notice'; message: string; anchor: Anchor; suggestion: string }
export function auditFindings(text: string, content: string, ranges?: TextRange[]): Finding[] {
  const values = jsonList(text, 'findings')
  const findings: Finding[] = values.flatMap(value => {
    if (!object(value)) return []
    const anchor = sourceAnchor(content, value.quote, value.occurrence, ranges)
    const { severity, message, suggestion } = value
    return anchor && (severity === 'severe' || severity === 'notice') && typeof message === 'string' && message.trim()
      && typeof suggestion === 'string' ? [{ anchor, severity, message, suggestion }] : []
  })
  if (findings.length !== values.length) throw new Error('部分审查意见缺少精确原文证据。')
  return findings
}

export function analysisContext(
  project: Project, chapter: Chapter, revision: Revision, profile: ModelProfile, prompt: string,
  canonVersions: Record<string, number> = {},
): FrozenContext {
  const stream = profile.capabilities.streaming !== 'verified_unsupported'
  return {
    schemaVersion: 1, capturedAt: new Date().toISOString(), projectId: project.projectId, baseRevisionId: revision.revisionId,
    previousRevisionIds: [revision.revisionId], chapterTitle: chapter.title, canonVersions, skillVersions: {},
    targetBeatChars: project.targetBeatChars, instruction: '', carryDraft: '', profileId: profile.id, profileVersion: profile.version,
    model: profile.model, rates: profile.rates, prompt,
    requestBody: { model: profile.model, messages: [{ role: 'user', content: prompt }], stream,
      [profile.tokenLimitField]: 4096, ...(stream ? { stream_options: { include_usage: true } } : {}),
      ...(profile.capabilities.jsonMode === 'verified_supported' ? { response_format: { type: 'json_object' } } : {}) },
  }
}

export function memoryAnalysis(
  mode: 'memory_extract' | 'consistency_check', project: Project, chapter: Chapter, revision: Revision,
  profile: ModelProfile, memory: Memory, povCharacterId?: string,
  sceneReview?: SceneReview,
  viewpoints?: ViewpointRange[],
) {
  const extraction = mode === 'memory_extract'
  const source = sceneReview?.ranges.map(range => revision.content.slice(range.start, range.end)).join('\n\n【下一段】\n\n') ?? revision.content
  const mentions = (terms: string[]) => terms.some(term => term.length > 0 && source.includes(term))
  const orderedCanon = [...memory.canon].sort((a, b) =>
    Number(mentions([b.name, ...b.aliases, ...b.audience.protectedTerms]))
      - Number(mentions([a.name, ...a.aliases, ...a.audience.protectedTerms]))
      || Number(b.locked) - Number(a.locked))
  const restrictedCanon = orderedCanon.filter(entry => entry.audience.scope !== 'public')
  const includedCanon = extraction ? orderedCanon.slice(0, 100)
    : [...restrictedCanon, ...orderedCanon.filter(entry => entry.audience.scope === 'public').slice(0, 100)]
  const canon = includedCanon.map(entry => extraction
    ? { id: entry.entryId, category: entry.category, name: entry.name, aliases: entry.aliases }
    : { ...entry, body: entry.audience.scope === 'public' ? Array.from(entry.body).slice(0, 260).join('') : entry.body })
  const knownEvents = memory.events.filter(event => isApproved(event) && event.current && event.chapterOrder <= chapter.chapterOrder)
  const newest = knownEvents.slice(-26)
  const linked = knownEvents.filter(event => event.subjects.some(id => orderedCanon.slice(0, 12)
    .some(entry => entry.entryId === id && mentions([entry.name, ...entry.aliases])))).slice(-22)
  const secrets = extraction ? [] : memory.events.filter(event => event.current && isApproved(event)
    && ((event.audience.scope !== 'public' && event.approval === 'author_accepted')
      || event.eventType === 'secret_created' || event.eventType === 'secret_revealed'
      || memory.knowledge.some(item => item.eventId === event.eventId)))
  const secretIds = new Set(secrets.map(event => event.eventId))
  const selected = new Map([...linked, ...newest, ...secrets].map(fact => [fact.eventId, fact]))
  const facts = [...selected.values()].map(({ eventId, summary, eventType, chapterOrder, audience, subjects, time }) =>
    ({ eventId, summary: secretIds.has(eventId) ? summary : Array.from(summary).slice(0, 240).join(''),
      eventType, chapterOrder, audience, subjects, time }))
  const visibleNotes = memory.notes.filter(item => ['open','reminded'].includes(item.status))
  const publicNotes = visibleNotes.filter(note => note.audience.scope === 'public').slice(-10)
    .map(note => ({ ...note, body: Array.from(note.body).slice(0, 220).join('') }))
  const prompt = [
    extraction ? '你是证据提取器。仅从给定的已确认原文提取事实，不使用常识补全。'
      : '你是旁白一致性审查员。下方包含作者全知数据。只审查事实冲突和视角泄密，勿续写或编辑正文。',
    extraction
      ? '只返回一个 JSON 对象 {"events":[]}。每项字段 eventType, subjects, summary, time, confidence, quote, occurrence, objectiveRoutine。'
        + 'subjects 只接受已有设定 ID。置信度 high/medium/low。quote 必须逐字引用原文，最长120字，重复时标明第几个 occurrence（从1起）。'
        + 'objectiveRoutine 只用于原文明确且普通的摘要、移动、次要人物或道具交接。死亡、秘密、关系变化与不可逆事件必须另列。'
        + '移动和道具交接若确实写在原文中且被本段的限知角色直接看到，可另列一条事件，summary 与 quote 使用完全相同的一句原文，勿改写。'
        + '类别：' + Object.keys(eventTypes).join(', ')
      : '只返回 JSON {"findings":[]}，每项为 severity(severe/notice)、message、suggestion、quote、occurrence。'
        + 'quote 必须逐字引用原文，最长120字，重复时标明 occurrence（从1起）。符合设定则返回空数组。',
    '设定：' + JSON.stringify(canon),
    extraction ? '' : '本场景视角人物 ID：' + (povCharacterId || '未指定') + '\n已采纳事实与知情状态：'
      + JSON.stringify({ events: facts, knowledge: memory.knowledge.filter(item => selected.has(item.eventId)),
        notes: [...visibleNotes.filter(note => note.audience.scope !== 'public'), ...publicNotes] }),
    !sceneReview && viewpoints?.length
      ? '本章按 UTF-16 位置划分的场景视角（此表优先于当前场景视角，空值表示该段未指定）：' + JSON.stringify(viewpoints) : '',
    (sceneReview ? '本场在本章的正文片段（引文出现次数在这些片段内计数）：\n' : '审查稿全文：\n') + source,
  ].filter(Boolean).join('\n\n')
  return { ...analysisContext(project, chapter, revision, profile, prompt,
    Object.fromEntries(includedCanon.map(entry => [entry.entryId, entry.version]))), sceneReview, viewpoints,
    memoryStamp: memory.stamp,
    viewpointAlerts: extraction ? undefined : protectedTermFindings(memory, chapter, revision.content, viewpoints?.length
      ? viewpoints : [{ start: 0, end: revision.content.length, povCharacterId: povCharacterId ?? null }]) }
}
