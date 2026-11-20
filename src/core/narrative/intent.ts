import type { Pack, Scene, SceneIntent, SkillAxis, StyleCatalog } from '../contracts/types'

const grief = /死亡|极度悲痛|丧事|哀悼|丧亲|葬礼|悼念|诀别|悲伤|重伤|生死危急/
const pressure = /紧迫|截止|倒计时|赶回|迫在眉睫|追赶|脱险|抢在/
const talk = /谈判|对白|对话|试探|交涉|质问|对峙/
const doubt = /悬疑|查证|调查|疑案|秘密|线索|反常/
const playful = /喜剧|包袱|轻松|玩笑|诙谐|窘态/

export function repeatedActions(recent: string) {
  const counts = new Map<string, number>()
  for (const match of recent.slice(-700).matchAll(/攥紧(?:拳头|拳)?|皱了皱眉|别过脸|摇了摇头|沉默片刻|倒吸一口气|嘴角抽了抽|心头一紧/g)) {
    counts.set(match[0], (counts.get(match[0]) ?? 0) + 1)
  }
  return [...counts].filter(([, count]) => count > 1).map(([action]) => action)
}

export function deriveIntent(scene: Scene | undefined, instruction: string, recent = '', direction = ''): SceneIntent {
  const context = [instruction, scene?.title, scene?.details.conflict].filter(Boolean).join(' ')
  const signal = context || [direction, recent.slice(-500)].filter(Boolean).join(' ')
  const inferred: SceneIntent = {
    goal: instruction.trim() || scene?.details.conflict || direction.split('\n')[0] || '完成一次能被下一拍承接的叙事推进',
    hiddenGoal: doubt.test(signal) ? '让人物发现可核对的异常，暂缓作出定论' : '',
    action: scene?.details.conflict || '接续最后一个行动，展示具体结果',
    emotion: grief.test(signal) ? '悲伤、克制' : doubt.test(signal) ? '谨慎、紧绷' : playful.test(signal) ? '轻松' : '延续前情',
    dialogue: talk.test(signal) ? '信息试探或利益交换' : '需要时用对话推进',
    pace: pressure.test(signal) ? '加速，按时点推进' : '随行动转折调整',
    humor: grief.test(signal) ? '禁止' : playful.test(signal) ? '允许' : '克制',
    pov: '延续已确认正文的限知视角',
  }
  const chosen = Object.fromEntries(Object.entries(scene?.details.intent ?? {}).filter(([, value]) => typeof value === 'string' && value.trim()))
  const combined: SceneIntent = { ...inferred, ...chosen }
  if (grief.test(context) || grief.test(combined.emotion)) combined.humor = '禁止'
  return combined
}

export function activeStyles(catalog: StyleCatalog, scene: Scene | undefined, intent: SceneIntent, instruction: string) {
  const enabled = catalog.packs.filter(pack => pack.enabled)
  const cartridge = enabled.find(pack => pack.itemId === catalog.activePackId && pack.slot === 'cartridge')
  const corpus = [instruction,scene?.title,scene?.details.conflict,...Object.values(intent),cartridge?.metadata.description].filter(Boolean).join(' ')
  const matchCount = (pack: Pack) => (pack.metadata.triggers ?? []).filter(trigger => corpus.includes(trigger)).length
  const allowed = (pack: Pack) => !(pack.metadata.antiTriggers ?? []).some(trigger => corpus.includes(trigger))
    && (pack.metadata.axis !== 'humor' || intent.humor === '允许')
  const baseChoices = enabled.filter(pack => pack.slot === 'base_style' && allowed(pack))
  const base = baseChoices.find(pack => pack.itemId === catalog.activeBaseSkillId)
    ?? [...baseChoices].sort((a, b) => matchCount(b) - matchCount(a))[0]
  const selected = scene?.details.techniqueIds ?? []
  const preferred = cartridge?.metadata.preferredSkills ?? []
  const defaultWeight: Partial<Record<SkillAxis, number>> = {
    narrative: 2, character: 1, emotion: grief.test(intent.emotion) ? 24 : 0, dialogue: talk.test(intent.dialogue) ? 20 : 0,
    humor: intent.humor === '允许' ? 15 : 0, pacing: pressure.test(intent.pace) ? 16 : 0,
  }
  const score = (pack: Pack) => (selected.includes(pack.itemId) ? 1000 : 0)
    + (preferred.includes(pack.itemId) ? 15 : 0) + 20 * matchCount(pack)
    + (defaultWeight[pack.metadata.axis ?? 'narrative'] ?? 0) + pack.priority / 10
  const techniques = enabled.filter(pack => pack.slot === 'technique' && allowed(pack))
    .filter(pack => selected.length === 0 || selected.includes(pack.itemId))
    .sort((a, b) => score(b) - score(a) || a.itemId.localeCompare(b.itemId)).slice(0, 2)
  return { cartridge, skills: [...(base ? [base] : []), ...techniques] }
}
