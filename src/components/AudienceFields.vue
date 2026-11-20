<script setup lang="ts">
import type { Audience, Canon } from '../core/narrative/memory'

const audience = defineModel<Audience>({ required: true })
defineProps<{ characters: Canon[] }>()
</script>

<template>
  <fieldset class="audience-fields">
    <legend>写作视角可见范围</legend>
    <label>知情范围
      <select v-model="audience.scope" @change="audience.visibleTo = []">
        <option value="narrator_only">仅作者与一致性审查</option>
        <option value="named_characters">指定人物</option>
        <option value="public">所有视角</option>
      </select>
    </label>
    <label v-if="audience.scope === 'named_characters'">知情人物
      <select v-model="audience.visibleTo" multiple required>
        <option v-for="person in characters" :key="person.entryId" :value="person.entryId">{{ person.name }}</option>
      </select>
    </label>
    <template v-if="audience.scope !== 'public'">
      <label>外人可观察到的线索<input v-model="audience.observableClue" maxlength="600"></label>
      <label>保密关键词（用顿号分隔）
        <input :value="audience.protectedTerms.join('、')" @change="audience.protectedTerms = ($event.target as HTMLInputElement).value.split(/[、，,]/).map(t => t.trim()).filter(Boolean)">
      </label>
    </template>
  </fieldset>
</template>
