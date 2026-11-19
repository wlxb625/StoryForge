import type { Rates, Trace } from '../contracts/types'

export function estimate(trace: Trace, rates?: Rates | null): [number, number] | null {
  if (trace.costUsd !== null) return [trace.costUsd, trace.costUsd]
  if (!rates || trace.promptTokens === null || trace.completionTokens === null) return null
  const cached = trace.cachedTokens
  if (cached != null && (cached < 0 || cached > trace.promptTokens)) return null
  const inputChoices = [rates.input, rates.cacheWrite ?? rates.input]
  if (cached == null) inputChoices.push(rates.cacheRead ?? rates.input)
  const known = trace.completionTokens * rates.output + (cached ?? 0) * (rates.cacheRead ?? rates.input)
  const remaining = trace.promptTokens - (cached ?? 0)
  return [(known + remaining * Math.min(...inputChoices)) / 1e6, (known + remaining * Math.max(...inputChoices)) / 1e6]
}
