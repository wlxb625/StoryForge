import type { Chapter, FrozenContext, ModelProfile, Project, PromptRevision, Revision, Scene, StyleCatalog } from '../contracts/types'
import { activeStyles, deriveIntent } from './intent'
import { emptyMemory, writerMemory, type Memory } from './memory'

export interface CreativeSources {
  scene?: Scene
  catalog?: StyleCatalog
  promptRevision?: PromptRevision
  memory?: Memory
  memoryMatches?: string[]
}

export function freezeContext(
  project: Project, chapter: Chapter, revision: Revision | undefined, profile: ModelProfile,
  instruction: string, carryDraft = '', sources?: CreativeSources,
): FrozenContext {
  const target = project.targetBeatChars
  const memory = writerMemory(sources?.memory ?? emptyMemory(), chapter, sources?.scene?.details.povCharacterId, sources?.memoryMatches)
  const authorInstruction = memory.safe(instruction)
  if (instruction.trim() && !authorInstruction) throw new Error('续写思路包含当前视角不可见的情节。请改用可观察的线索。')
  const scene = memory.guarded ? undefined : sources?.scene
  const priorText = memory.guarded ? '' : revision?.content
  const intent = deriveIntent(scene, authorInstruction, priorText)
  if (memory.povName) intent.pov = '固定于角色「' + memory.povName + '」'
  const { skills, cartridge } = activeStyles(sources?.catalog ?? { packs: [], activeBaseSkillId: null, activePackId: null }, scene, intent, instruction)
  const activeSkills = [...(cartridge ? [cartridge] : []), ...skills].map(pack => ({ id: pack.itemId, name: pack.name, version: pack.version }))
  const sceneLabel = scene ? [
    '当前场景：' + scene.title, scene.details.time && '时间：' + scene.details.time,
    scene.details.location && '地点：' + scene.details.location, scene.details.cast && '在场人物：' + scene.details.cast,
    scene.details.conflict && '矛盾：' + scene.details.conflict,
  ].filter(Boolean).join('\n') : ''
  const prompt = [
    '你正在协助创作一部中文长篇小说。续写当前章节的一个节拍，只返回新正文。',
    memory.safe(`作品：${project.title}\n章节：${chapter.title}`),
    '优先级：知情边界与既有事实最高，其次是作者要求和场景情绪，最后应用文笔技法。',
    `视角边界：${intent.pov}。只写视角人物能观察、回忆或合理推断的内容。`,
    sceneLabel,
    `节拍参考 ${Math.round(target * .78)}–${Math.round(target * 1.22)} 个汉字。写出一次有因果的变化，保留同一视角和既定事实。`,
    authorInstruction.trim() ? `作者指令：${authorInstruction.trim()}` : '沿用可见前情，推进一个新行动。',
    !memory.guarded && sources?.promptRevision?.instruction ? '作者提示词覆写：' + sources.promptRevision.instruction : '',
    `戏剧目的：${intent.goal}${intent.hiddenGoal ? '。潜在目的：' + intent.hiddenGoal : ''}\n表层行动：${intent.action}\n内在情绪：${intent.emotion}\n交流状态：${intent.dialogue}\n节奏：${intent.pace}\n幽默许可：${intent.humor}`,
    memory.context,
    cartridge ? memory.safe(`题材骨架：${cartridge.name} · ${cartridge.body}\n路标：${cartridge.metadata.stages?.join(' / ') ?? ''}`) : '',
    priorText ? `已确认正文末尾：\n${Array.from(priorText).slice(-6000).join('')}` : '',
    !memory.guarded && carryDraft ? `作者选择从这段草稿末尾接续。不要重写这段草稿：\n${Array.from(carryDraft).slice(-6000).join('')}` : '',
    ...skills.map(pack => memory.safe(`文笔参考 · ${pack.name}（沿用动作规则，另写人物、物品和台词）\n${pack.body}\n避免套话：${pack.metadata.avoidPhrases?.join('、') ?? ''}`)),
  ].filter(Boolean).join('\n\n')
  const streaming = profile.capabilities.streaming !== 'verified_unsupported'
  return {
    schemaVersion: 1, capturedAt: new Date().toISOString(), projectId: project.projectId,
    baseRevisionId: chapter.currentRevisionId,
    previousRevisionIds: priorText && revision ? [revision.revisionId] : [], chapterTitle: memory.safe(chapter.title),
    canonVersions: memory.versions, skillVersions: Object.fromEntries(activeSkills.map(skill => [skill.id, skill.version])),
    memoryStamp: sources?.memory?.stamp,
    targetBeatChars: target, instruction: instruction.trim(), carryDraft, scene, intent, activeSkills,
    promptRevisionId: sources?.promptRevision?.promptRevisionId, rates: profile.rates,
    profileId: profile.id, profileVersion: profile.version, model: profile.model, prompt,
    requestBody: {
      model: profile.model, messages: [{ role: 'user', content: prompt }], stream: streaming,
      [profile.tokenLimitField]: 4096,
      ...(streaming ? { stream_options: { include_usage: true } } : {}),
    },
  }
}

export function retarget(context: FrozenContext, profile: ModelProfile): FrozenContext {
  const stream = profile.capabilities.streaming !== 'verified_unsupported'
  return {
    ...context, profileId: profile.id, profileVersion: profile.version, model: profile.model, rates: profile.rates,
    requestBody: { model: profile.model, messages: [{ role: 'user', content: context.prompt }], stream,
      [profile.tokenLimitField]: 4096, ...(stream ? { stream_options: { include_usage: true } } : {}) },
  }
}
