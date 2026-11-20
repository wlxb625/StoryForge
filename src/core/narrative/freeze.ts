import type { Chapter, FrozenContext, ModelProfile, Project, PromptRevision, Revision, Scene, StyleCatalog } from '../contracts/types'
import { activeStyles, deriveIntent, repeatedActions } from './intent'
import { emptyMemory, writerMemory, type Memory } from './memory'

export interface CreativeSources {
  scene?: Scene
  catalog?: StyleCatalog
  promptRevision?: PromptRevision
  memory?: Memory
  memoryMatches?: string[]
  direction?: string
  arcTitle?: string
  previous?: Revision
  currentSceneText?: string
  includeGuardedCarry?: boolean
  mode?: 'inspiration'
}

export function freezeContext(
  project: Project, chapter: Chapter, revision: Revision | undefined, profile: ModelProfile,
  instruction: string, carryDraft = '', sources?: CreativeSources,
): FrozenContext {
  const target = project.targetBeatChars
  const memory = writerMemory(sources?.memory ?? emptyMemory(), chapter, sources?.scene?.details.povCharacterId, sources?.memoryMatches)
  const authorInstruction = memory.safe(instruction)
  if (instruction.trim() && !authorInstruction) throw new Error('续写思路包含当前视角不可见的情节。请改用可观察的线索。')
  const selectedScene = sources?.scene
  const scene = selectedScene && {
    ...selectedScene, title: memory.safe(selectedScene.title), details: {
      ...selectedScene.details,
      time: memory.safe(selectedScene.details.time), location: memory.safe(selectedScene.details.location),
      cast: memory.safe(selectedScene.details.cast), conflict: memory.safe(selectedScene.details.conflict),
      intent: Object.fromEntries(Object.entries(selectedScene.details.intent ?? {}).flatMap(([key, value]) =>
        value && memory.safe(value) ? [[key, value]] : [])),
    },
  }
  const priorText = memory.guarded ? '' : sources?.currentSceneText ?? revision?.content
  const previous = memory.guarded ? undefined : sources?.previous
  const direction = memory.safe(sources?.direction ?? '')
  const catalog = sources?.catalog ?? { packs: [], activeBaseSkillId: null, activePackId: null }
  const activeCartridge = catalog.packs.find(pack => pack.enabled && pack.slot === 'cartridge' && pack.itemId === catalog.activePackId)
  const stages = activeCartridge?.metadata.stages ?? []
  const assigned = scene?.details.cartridgeStage?.packId === activeCartridge?.itemId ? scene?.details.cartridgeStage?.stage : undefined
  const cartridgeStage = stages.find(stage => memory.safe(stage) && (stage === assigned || (!assigned && stage === sources?.arcTitle)))
  const cartridgeBrief = activeCartridge && memory.safe(`题材骨架：${activeCartridge.name} · ${activeCartridge.body}`)
  const progression = cartridgeStage ? `本场卡带路标：${cartridgeStage}\n本拍只完成这个路标下的一次行动或验证，保留可留待后续兑现的结果。` : ''
  const intent = deriveIntent(scene, authorInstruction, priorText || previous?.content, direction || cartridgeStage)
  if (memory.povName) intent.pov = '固定于角色「' + memory.povName + '」'
  const { skills } = activeStyles(catalog, scene, intent, authorInstruction)
  const repetition = repeatedActions(priorText ?? previous?.content ?? '')
  const techniquePrompts = skills.flatMap(pack => {
    const text = memory.safe(`文笔参考 · ${pack.name}（沿用动作规则，另写人物、物品和台词）\n${pack.body}\n避免套话：${pack.metadata.avoidPhrases?.join('、') ?? ''}`)
    return text ? [{ pack, text }] : []
  })
  const activeSkills = [...(cartridgeBrief && activeCartridge ? [activeCartridge] : []), ...techniquePrompts.map(item => item.pack)]
    .map(pack => ({ id: pack.itemId, name: pack.name, version: pack.version }))
  const sceneLabel = scene ? [
    scene.title && '当前场景：' + scene.title, scene.details.time && '时间：' + scene.details.time,
    scene.details.location && '地点：' + scene.details.location, scene.details.cast && '在场人物：' + scene.details.cast,
    scene.details.conflict && '矛盾：' + scene.details.conflict,
  ].filter(Boolean).join('\n') : ''
  const prompt = [
    sources?.mode === 'inspiration'
      ? '你正在协助创作一部中文长篇小说。提出三个不同的下一拍行动供作者选择。每行一条，不加序号，每条不超过 80 字。用当前人物可观察的线索，不写续稿，不编造既有事实。'
      : '你正在协助创作一部中文长篇小说。续写当前章节的一个节拍，只返回新正文。',
    memory.safe(`作品：${project.title}\n章节：${chapter.title}`),
    '优先级：知情边界与既有事实最高，其次是作者要求和场景情绪，最后应用文笔技法。',
    `视角边界：${intent.pov}。只写视角人物能观察、回忆或合理推断的内容。`,
    sceneLabel,
    sources?.mode === 'inspiration' ? '' : `节拍参考 ${Math.round(target * .78)}–${Math.round(target * 1.22)} 个汉字。写出一次有因果的变化，保留同一视角和既定事实。`,
    authorInstruction.trim() ? `作者指令：${authorInstruction.trim()}` : '沿用可见前情，推进一个新行动。',
    !memory.guarded && sources?.promptRevision?.instruction ? '作者提示词覆写：' + sources.promptRevision.instruction : '',
    `戏剧目的：${intent.goal}${intent.hiddenGoal ? '。潜在目的：' + intent.hiddenGoal : ''}\n表层行动：${intent.action}\n内在情绪：${intent.emotion}\n交流状态：${intent.dialogue}\n节奏：${intent.pace}\n幽默许可：${intent.humor}`,
    memory.context,
    repetition.length ? `最近已反复出现「${repetition.join('、')}」，有合适的变化时可换一种动作。` : '',
    direction,
    previous ? `上一章正文末尾：\n${Array.from(previous.content).slice(-1800).join('')}` : '',
    cartridgeBrief, progression,
    priorText ? `已确认正文末尾：\n${Array.from(priorText).slice(-6000).join('')}` : '',
    carryDraft && (!memory.guarded || (sources?.includeGuardedCarry && memory.safe(carryDraft))) ? `从本场暂存稿的末尾接续。不要重写这段草稿：\n${Array.from(carryDraft).slice(-6000).join('')}` : '',
    ...techniquePrompts.map(item => item.text),
  ].filter(Boolean).join('\n\n')
  const streaming = profile.capabilities.streaming !== 'verified_unsupported'
  return {
    schemaVersion: 1, capturedAt: new Date().toISOString(), projectId: project.projectId,
    baseRevisionId: chapter.currentRevisionId,
    previousRevisionIds: [...(previous ? [previous.revisionId] : []), ...(priorText && revision ? [revision.revisionId] : [])],
    chapterTitle: memory.safe(chapter.title),
    canonVersions: memory.versions, skillVersions: Object.fromEntries(activeSkills.map(skill => [skill.id, skill.version])),
    memoryStamp: sources?.memory?.stamp, memoryEvidence: memory.evidence, cartridgeStage,
    targetBeatChars: target, instruction: instruction.trim(), carryDraft, scene, intent, activeSkills,
    sceneBindingId: sources?.scene?.sceneId ?? null, sceneBindingVersion: sources?.scene?.version,
    promptRevisionId: sources?.promptRevision?.promptRevisionId, rates: profile.rates,
    profileId: profile.id, profileVersion: profile.version, model: profile.model, prompt,
    requestBody: {
      model: profile.model, messages: [{ role: 'user', content: prompt }], stream: streaming,
      [profile.tokenLimitField]: sources?.mode === 'inspiration' ? 512 : 4096,
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
