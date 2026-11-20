<script setup lang="ts">
import type { FrozenContext } from '../core/contracts/types'

defineProps<{ context: FrozenContext }>()
const emit = defineEmits<{ open: [chapterId: string, quote: string, offset: number] }>()
</script>

<template>
  <details v-if="context.memoryEvidence || context.activeSkills?.length" class="writing-context">
    <summary>写作依据<span v-if="context.memoryEvidence">{{ context.memoryEvidence.length }} 条历史证据</span></summary>
    <p v-if="context.activeSkills?.length" class="muted">{{ context.activeSkills.map(skill => skill.name + ' ' + skill.version).join(' · ') }}</p>
    <p v-if="context.cartridgeStage">卡带路标 · {{ context.cartridgeStage }}</p>
    <article v-for="fact in context.memoryEvidence" :key="fact.eventId" class="context-fact">
      <small class="muted">第 {{ fact.chapterOrder }} 章</small>
      <p>{{ fact.summary }}</p>
      <blockquote v-if="fact.anchor.quote !== fact.summary">{{ fact.anchor.quote }}</blockquote>
      <button class="flat-button" type="button" @click="emit('open', fact.chapterId, fact.anchor.quote, fact.anchor.startOffset)">定位原文</button>
    </article>
    <p v-if="context.memoryEvidence?.length === 0" class="muted">本次没有可用的历史事实。可在记忆库确认原文证据和知情范围。</p>
  </details>
</template>
