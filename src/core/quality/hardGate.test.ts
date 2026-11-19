import { expect, it } from 'vitest'
import { countHan, technicalIssue } from './hardGate'

it('counts CJK and rejects technical failures without scoring literary style', () => {
  expect(countHan('钥匙。a 𠀀')).toBe(3)
  expect(technicalIssue(' ', true)).toMatch(/未返回/)
  expect(technicalIssue('她抬手', true, 'length')).toMatch(/截断/)
  expect(technicalIssue('data: {"choices":[]}')).toMatch(/协议/)
  const window = '八个汉字测试段落'
  expect(technicalIssue(window.repeat(2))).toBeNull()
  expect(technicalIssue(window.repeat(3))).toMatch(/复读/)
  expect(technicalIssue('他的心猛烈地跳了一下，胸口发紧。', true, 'stop')).toBeNull()
})
