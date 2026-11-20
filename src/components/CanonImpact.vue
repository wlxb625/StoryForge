<script setup lang="ts">
import type { CanonImpact } from '../core/narrative/memory'

defineProps<{ impact: CanonImpact }>()
</script>

<template>
  <div class="canon-impact">
    <p>{{ impact.hits.length }} 章正文 · {{ impact.facts.length }} 条事件 · {{ impact.notes.length }} 条叙事备注</p>
    <ul v-if="impact.hits.length || impact.facts.length || impact.notes.length">
      <li v-for="hit in impact.hits" :key="hit.chapterId">第 {{ hit.chapterOrder }} 章 · {{ hit.title }}<small>{{ hit.excerpt }}</small></li>
      <li v-for="fact in impact.facts" :key="fact.eventId">事件 · {{ fact.summary }}</li>
      <li v-for="note in impact.notes" :key="note.noteId">备注 · {{ note.body }}</li>
    </ul>
    <p v-else class="muted">未检索到关联正文、事件或备注。</p>
  </div>
</template>
