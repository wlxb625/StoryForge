const han = /\p{Script=Han}/gu

export const countHan = (content: string) => content.match(han)?.length ?? 0

export function technicalIssue(text: string, isFinal = false, finishReason?: string | null): string | null {
  if (isFinal && !text.trim()) return '模型未返回正文。'
  if (finishReason === 'length') return '模型达到输出上限，正文可能截断。'
  if (finishReason === 'content_filter' || finishReason === 'tool_calls' || finishReason === 'function_call')
    return '模型以非正文状态结束。'
  if (/^(?:\s*data:\s*\{|\s*\{\s*"(?:choices|usage|error)"\s*:|\s*<think>)/u.test(text))
    return '响应包含模型协议或思考标签。'
  const tail = (text.match(han) ?? []).slice(-300)
  const seen = new Map<string, { index: number; count: number }>()
  for (let index = 0; index + 8 <= tail.length; index++) {
    const key = tail.slice(index, index + 8).join('')
    const previous = seen.get(key)
    if (previous && index < previous.index + 8) continue
    const count = (previous?.count ?? 0) + 1
    if (count >= 3) return '最近正文中检测到连续高置信复读。'
    seen.set(key, { index, count })
  }
  return null
}
