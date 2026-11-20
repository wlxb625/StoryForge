import { expect, it } from 'vitest'
import type { Revision } from '../contracts/types'
import { sceneRanges } from './scenes'
import { sourceAnchor } from './memory'

function revision(revisionId: string, parentRevisionId: string | null, sceneId: string, content: string): Revision {
  return { revisionId, parentRevisionId, sceneId, content, saveKind: 'formal_revision', sourceTag: 'manual_edit', cjkCharCount: 0, createdAt: '' }
}

it('retains multiple scene bindings, including after edits and revisits', () => {
  const first = revision('1', null, 'gate', '门开了。')
  const next = revision('2', '1', 'hall', first.content + '门开了。走进大厅。')
  const edit = revision('3', '2', 'hall', '门慢慢开了。门开了。走进大厅。')
  const revisit = revision('4', '3', 'gate', edit.content + '门口雨停了。')
  const ranges = sceneRanges([revisit, edit, next, first], revisit)
  expect(ranges.map(range => [range.sceneId, revisit.content.slice(range.start, range.end)])).toEqual([
    ['gate', '门慢慢开了。'], ['hall', '门开了。走进大厅。'], ['gate', '门口雨停了。'],
  ])
  const gate = ranges.filter(range => range.sceneId === 'gate')
  expect(sourceAnchor(revisit.content, '门开了。', undefined, gate)).toBeUndefined()
  expect(sourceAnchor(revisit.content, '门口雨停了。', 1, gate)?.startOffset).toBe(edit.content.length)
})
