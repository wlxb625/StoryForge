import { parseDocument, stringify } from 'yaml'
import type { Pack, SkillAxis } from '../contracts/types'

const axes = new Set(['narrative', 'character', 'emotion', 'dialogue', 'humor', 'pacing'])

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error('配方需要一个声明式对象。')
  return value as Record<string, unknown>
}
function text(value: unknown, label: string, limit = 120): string {
  if (typeof value !== 'string' || !value.trim() || value.length > limit) throw new Error('请填写有效的' + label + '。')
  return value.trim()
}
function texts(value: unknown, label: string, limit = 30): string[] {
  if (value == null) return []
  if (!Array.isArray(value) || value.length > limit) throw new Error(label + '应为简短文本列表。')
  return value.map(item => text(item, label, 400))
}
function identity(header: Record<string, unknown>) {
  const itemId = text(header.id, '配方标识')
  const name = text(header.name, '配方名称')
  const version = text(header.version, '版本', 40)
  if (!/^[a-z0-9][a-z0-9._-]*$/i.test(itemId) || !/^\d+\.\d+\.\d+(?:-[a-z0-9.-]+)?$/i.test(version))
    throw new Error('标识应使用字母、数字或点号，版本应为 1.0.0 格式。')
  return { itemId, name, version }
}

function verifyPlain(source: string) {
  if (/<script\b|javascript:|sk-[a-z0-9_-]{20,}|(?:api[_ -]?key|authorization)\s*:\s*[^\s]+/i.test(source))
    throw new Error('配方只能包含声明式文本，请移除脚本或凭据。')
  if (/^\x60{3}(?:javascript|js|ts|python|py|sh|bash|powershell|rust)\b/im.test(source))
    throw new Error('配方不接受可执行代码示例。')
  if (source.length > 150000) throw new Error('配方文件太大。')
}

export function parseSkill(source: string): Pack {
  const raw = source.replace(/^\uFEFF/, '').trim()
  verifyPlain(raw)
  const match = /^---\s*\r?\n([\s\S]+?)\r?\n---[ \t]*\r?\n([\s\S]+)$/.exec(raw)
  if (!match?.[1] || !match[2]) throw new Error('Writing Skill 需有 YAML Frontmatter 和 Markdown 正文。')
  const header = parseDocument(match[1], { schema: 'core', uniqueKeys: true })
  if (header.errors.length) throw new Error('Writing Skill 的 YAML 格式有误。')
  const config = object(header.toJS({ maxAliasCount: 0 }))
  const slot = config.slot
  if (slot !== 'base_style' && slot !== 'technique') throw new Error('请选择 base_style 或 technique 槽位。')
  const priority = config.priority
  if (typeof priority !== 'number' || !Number.isInteger(priority) || priority < 1 || priority > 100)
    throw new Error('技能优先级需为 1 到 100。')
  if (config.axis != null && (typeof config.axis !== 'string' || !axes.has(config.axis))) throw new Error('技能类别无效。')
  const body = match[2].trim()
  const names = [...body.matchAll(/^##[ \t]+(核心技法|正例|反例)(?:[ \t（(].*)?$/gm)].map(m => m[1])
  const sections = body.split(/^##[ \t]+(?:核心技法|正例|反例).*$\r?\n/gm).slice(1)
  function blocks(section?: string) { return section?.trim().split(/\r?\n\s*\r?\n/).filter(Boolean) ?? [] }
  const positives = blocks(sections[1])
  const negatives = blocks(sections[2])
  if (names.join('|') !== '核心技法|正例|反例' || !sections[0]?.trim()
      || positives.length !== 2 || !positives.every((item) => item.startsWith('> '))
      || negatives.length < 1 || negatives.length > 2 || !negatives.every((item) => item.startsWith('> ') && item.includes('错因')))
    throw new Error('技能需按顺序包含核心技法、两段引用式正例和一到两段附错因的反例。')
  return {
    ...identity(config), slot, itemKind: 'writing_skill', priority, body, enabled: true,
    metadata: {
      axis: (config.axis as SkillAxis | undefined) ?? 'narrative',
      triggers: texts(config.triggers, '触发词'), antiTriggers: texts(config.anti_triggers, '排除词'),
      avoidPhrases: texts(config.avoid_phrases, '避雷词', 15),
    },
  }
}

export function parseCartridge(source: string): Pack {
  verifyPlain(source)
  const raw = object(JSON.parse(source.replace(/^\uFEFF/, '')))
  const description = text(raw.description ?? raw.summary, '题材说明', 5000)
  const stages = texts(raw.stages, '情节路标')
  return {
    ...identity(raw), itemKind: 'genre_cartridge', slot: 'cartridge', priority: 20,
    metadata: { description, stages, preferredSkills: texts(raw.preferred_skills, '推荐技能') }, body: description, enabled: true,
  }
}

export function serializeSkill(pack: Pack) {
  return '---\n' + stringify({
    id: pack.itemId, name: pack.name, version: pack.version, slot: pack.slot, priority: pack.priority,
    axis: pack.metadata.axis, triggers: pack.metadata.triggers ?? [], anti_triggers: pack.metadata.antiTriggers ?? [],
    avoid_phrases: pack.metadata.avoidPhrases ?? [],
  }) + '---\n' + pack.body + '\n'
}

export function serializePack(pack: Pack) {
  if (pack.slot !== 'cartridge') {
    const contents = serializeSkill(pack)
    parseSkill(contents)
    return { name: pack.itemId + '.md', contents }
  }
  const contents = JSON.stringify({
    id: pack.itemId, name: pack.name, version: pack.version, description: pack.metadata.description ?? pack.body,
    stages: pack.metadata.stages ?? [], preferred_skills: pack.metadata.preferredSkills ?? [],
  }, null, 2) + '\n'
  parseCartridge(contents)
  return { name: pack.itemId + '.json', contents }
}

export function unwrapSkillDraft(text: string) {
  const trimmed = text.trim()
  return /^```(?:markdown|md)?\s*\n([\s\S]+?)\n```$/i.exec(trimmed)?.[1] ?? trimmed
}
