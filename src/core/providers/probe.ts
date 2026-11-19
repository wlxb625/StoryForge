import type { Capabilities, ModelProfile, ProbeResult, Verification } from '../contracts/types'
import { desktop } from '../db/desktop'

function disposition(result: ProbeResult, verified: boolean): Verification {
  if (verified) return 'verified_supported'
  if (result.state === 'completed' || result.status === 400 || result.status === 422) return 'verified_unsupported'
  return 'unknown'
}

export async function probeProfile(profile: ModelProfile): Promise<Capabilities> {
  const run = (stream: boolean) => desktop.probe(profile.id, {
    model: profile.model,
    messages: [{ role: 'user', content: stream ? '只输出 OK。' : '只输出 JSON 对象 {"ok":true}。' }],
    stream,
    ...(stream ? { stream_options: { include_usage: true } } : { response_format: { type: 'json_object' } }),
    [profile.tokenLimitField]: 128,
  })
  const streaming = await run(true)
  if (streaming.status === 401 || streaming.status === 403 || streaming.status === 404)
    throw new Error(streaming.message ?? '模型探测未能连接')
  const json = await run(false)
  const capabilities: Capabilities = {
    streaming: disposition(streaming, streaming.streamed && streaming.state === 'completed'),
    jsonMode: disposition(json, json.state === 'completed' && isProbeJson(json.content)),
    usage: streaming.usageObserved || json.usageObserved ? 'verified_supported' : 'unknown',
    cancel: 'declared', contextLen: 'unknown', multimodal: 'unknown',
    promptCache: streaming.hasCachedTokens || json.hasCachedTokens ? 'verified_supported' : 'unknown',
  }
  return desktop.saveCapabilities(profile, capabilities)
}

export function isProbeJson(text: string): boolean {
  try {
    return JSON.parse(text)?.ok === true
  } catch {
    return false
  }
}
