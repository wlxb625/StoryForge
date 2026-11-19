import type { Chapter, FrozenContext, ModelProfile, Project, Revision } from '../contracts/types'

export function freezeContext(
  project: Project, chapter: Chapter, revision: Revision | undefined, profile: ModelProfile,
  instruction: string, carryDraft = '',
): FrozenContext {
  const target = project.targetBeatChars
  const prompt = [
    '你正在协助创作一部中文长篇小说。续写当前章节的一个节拍，只返回新正文。',
    `作品：${project.title}\n章节：${chapter.title}`,
    `节拍参考 ${Math.round(target * .78)}–${Math.round(target * 1.22)} 个汉字。写出一次有因果的变化，保留同一视角和既定事实。`,
    instruction.trim() ? `作者指令：${instruction.trim()}` : '从现有正文的最后一个行动继续。',
    revision?.content ? `已确认正文末尾：\n${Array.from(revision.content).slice(-6000).join('')}` : '',
    carryDraft ? `作者选择从这段草稿末尾接续。不要重写这段草稿：\n${Array.from(carryDraft).slice(-6000).join('')}` : '',
  ].filter(Boolean).join('\n\n')
  const streaming = profile.capabilities.streaming !== 'verified_unsupported'
  return {
    schemaVersion: 1, capturedAt: new Date().toISOString(), projectId: project.projectId,
    baseRevisionId: chapter.currentRevisionId,
    previousRevisionIds: revision ? [revision.revisionId] : [], chapterTitle: chapter.title,
    canonVersions: {}, skillVersions: {}, targetBeatChars: target, instruction: instruction.trim(), carryDraft,
    profileId: profile.id, profileVersion: profile.version, model: profile.model, prompt,
    requestBody: {
      model: profile.model, messages: [{ role: 'user', content: prompt }], stream: streaming,
      [profile.tokenLimitField]: 4096,
      ...(streaming ? { stream_options: { include_usage: true } } : {}),
    },
  }
}
