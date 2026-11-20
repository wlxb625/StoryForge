import type { Revision } from '../contracts/types'

export interface TextRange { start: number; end: number }
export interface SceneRange extends TextRange { sceneId: string | null }
export interface SceneReview { sceneId: string; ranges: TextRange[] }
export interface ViewpointRange extends TextRange { povCharacterId: string | null }

function splitsCharacter(text: string, at: number) {
  return at > 0 && at < text.length && /[\uD800-\uDBFF]/.test(text[at - 1]!) && /[\uDC00-\uDFFF]/.test(text[at]!)
}

export function sceneRanges(history: Revision[], head: Revision): SceneRange[] {
  const lookup = new Map(history.map(revision => [revision.revisionId, revision]))
  const chain = [head]
  const seen = new Set([head.revisionId])
  for (let parent = head.parentRevisionId; parent && !seen.has(parent); ) {
    const revision = lookup.get(parent)
    if (!revision) break
    seen.add(parent)
    chain.push(revision)
    parent = revision.parentRevisionId
  }
  let before = ''
  let ranges: SceneRange[] = []
  for (const revision of chain.reverse()) {
    const after = revision.content
    if (before === after) continue
    let start = 0
    while (start < Math.min(before.length, after.length) && before[start] === after[start]) start++
    if (splitsCharacter(before, start) || splitsCharacter(after, start)) start--
    let suffix = 0
    while (suffix < Math.min(before.length, after.length) - start && before.at(-suffix - 1) === after.at(-suffix - 1)) suffix++
    if (splitsCharacter(before, before.length - suffix) || splitsCharacter(after, after.length - suffix)) suffix--
    const oldEnd = before.length - suffix
    const end = after.length - suffix
    const shift = end - oldEnd
    const owner = ranges.find(range => oldEnd > start
      ? start >= range.start && oldEnd <= range.end
      : start > range.start && start < range.end)
    const next = [
      ...ranges.filter(range => range.start < start).map(range => ({ ...range, end: Math.min(range.end, start) })),
      ...(end > start ? [{ start, end, sceneId: owner ? owner.sceneId : revision.sceneId ?? null }] : []),
      ...ranges.filter(range => range.end > oldEnd).map(range => ({
        ...range, start: Math.max(range.start, oldEnd) + shift, end: range.end + shift,
      })),
    ]
    ranges = []
    for (const range of next) {
      const last = ranges.at(-1)
      if (last && last.end === range.start && last.sceneId === range.sceneId) last.end = range.end
      else ranges.push(range)
    }
    before = after
  }
  return ranges
}
