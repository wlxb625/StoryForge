<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import { KeyRound, Plus, PlugZap, Trash2, X } from 'lucide-vue-next'
import { errorMessage, type ModelProfile, type ProfileInput } from '../core/contracts/types'
import { desktop } from '../core/db/desktop'
import { probeProfile } from '../core/providers/probe'
import { useStudio } from '../stores/studio'

defineEmits<{ close: [] }>()
const studio = useStudio()
const form = reactive<ProfileInput>({
  name: '', model: '', baseUrl: '', needsKey: true, direct: false, tokenLimitField: 'max_tokens',
})
const pending = ref(false)
const rates = reactive({ input: '', output: '', cacheRead: '', cacheWrite: '' })
const warning = ref('')
const active = computed(() => studio.profiles.find((profile) => profile.id === form.id))
const labels = {
  streaming: '流式响应', jsonMode: 'JSON 模式', usage: '真实用量', cancel: '本机取消',
  contextLen: '上下文长度', promptCache: '缓存统计', multimodal: '多模态',
} as const
const states = {
  verified_supported: '已验证', verified_unsupported: '不支持', declared: '已声明', unknown: '未知',
} as const

function choose(profile?: ModelProfile) {
  warning.value = ''
  Object.assign(form, profile ? {
    id: profile.id, name: profile.name, model: profile.model, baseUrl: profile.baseUrl, needsKey: profile.needsKey,
    direct: profile.direct, tokenLimitField: profile.tokenLimitField,
  } : { id: undefined, name: '', model: '', baseUrl: '', needsKey: true, direct: false, tokenLimitField: 'max_tokens' })
  for (const key of Object.keys(rates) as Array<keyof typeof rates>) {
    const value = profile?.rates?.[key]
    rates[key] = value == null ? '' : String(value)
  }
}
choose(studio.selectedProfile ?? studio.profiles[0])

async function act(operation: () => Promise<void>) {
  if (pending.value) return
  pending.value = true
  warning.value = ''
  try { await operation() } catch (error) { warning.value = errorMessage(error) }
  finally { pending.value = false }
}

function save() {
  return act(async () => {
    const hasRates = Object.values(rates).some(value => !!value.trim())
    if (hasRates && (!rates.input.trim() || !rates.output.trim())) throw new Error('填写计费信息时，输入和输出单价均为必填。')
    const saved = await desktop.saveProfile({
      id: form.id, name: form.name, model: form.model, baseUrl: form.baseUrl,
      needsKey: form.needsKey, direct: form.direct, tokenLimitField: form.tokenLimitField,
      rates: hasRates ? { input: Number(rates.input), output: Number(rates.output),
        cacheRead: rates.cacheRead.trim() ? Number(rates.cacheRead) : null, cacheWrite: rates.cacheWrite.trim() ? Number(rates.cacheWrite) : null } : null,
    })
    await studio.readProfiles()
    studio.profileId = saved.id
    choose(studio.profiles.find((profile) => profile.id === saved.id))
    studio.flash('模型档案已保存')
  })
}
function credential() {
  const profile = active.value
  if (!profile) return
  return act(async () => {
    if (await desktop.setSecret(profile.id)) {
      await studio.readProfiles()
      studio.flash('密钥已存入 Windows 凭据库')
    }
  })
}
function remove() {
  const profile = active.value
  if (!profile || !window.confirm(`删除本机模型「${profile.name}」及其系统凭据？`)) return
  return act(async () => {
    await desktop.removeProfile(profile.id)
    await studio.readProfiles()
    choose(studio.selectedProfile)
  })
}
function probe() {
  const profile = active.value
  if (!profile) return
  return act(async () => {
    await probeProfile(profile)
    await studio.readProfiles()
    studio.flash('模型能力已更新')
  })
}
</script>

<template>
  <div class="dialog-shade" @click.self="$emit('close')" @keydown.esc="$emit('close')">
    <section class="model-dialog" role="dialog" aria-modal="true" aria-labelledby="model-title">
      <header class="dialog-header">
        <div><p class="micro">本机设置</p><h2 id="model-title">模型档案</h2></div>
        <button type="button" class="icon-button" aria-label="关闭模型设置" @click="$emit('close')"><X :size="19" /></button>
      </header>
      <div class="model-body">
        <nav class="profile-list" aria-label="已配置的模型">
          <button class="flat-button new-model" type="button" :disabled="pending" @click="choose()"><Plus :size="16" /> 添加模型</button>
          <button v-for="profile in studio.profiles" :key="profile.id" class="profile-entry" :class="{ selected: form.id === profile.id }"
            type="button" :disabled="pending" @click="choose(profile)">
            <strong>{{ profile.name }}</strong><small>{{ profile.model }}</small>
          </button>
        </nav>
        <form class="model-form" @submit.prevent="save">
          <label>显示名称<input v-model="form.name" required placeholder="如 主笔模型" maxlength="80"></label>
          <label>Base URL<input v-model="form.baseUrl" required type="url" placeholder="https://服务地址/v1"></label>
          <label>模型 ID<input v-model="form.model" required placeholder="填写服务给出的完整模型名" maxlength="200"></label>
          <div class="model-options">
            <label><input v-model="form.needsKey" type="checkbox"> 需要 API 密钥</label>
            <label><input v-model="form.direct" type="checkbox"> 直连服务</label>
          </div>
          <label>输出上限参数
            <select v-model="form.tokenLimitField">
              <option value="max_tokens">max_tokens</option>
              <option value="max_completion_tokens">max_completion_tokens</option>
            </select>
          </label>
          <details class="rate-inputs">
            <summary>计费单价（美元／百万 token）</summary>
            <label>输入<input v-model="rates.input" type="text" inputmode="decimal"></label>
            <label>输出<input v-model="rates.output" type="text" inputmode="decimal"></label>
            <label>缓存读<input v-model="rates.cacheRead" type="text" inputmode="decimal"></label>
            <label>缓存写<input v-model="rates.cacheWrite" type="text" inputmode="decimal"></label>
          </details>
          <div class="model-actions">
            <button class="primary-button" type="submit" :disabled="pending">保存档案</button>
            <button v-if="active?.needsKey" class="secondary-button" type="button" :disabled="pending" @click="credential">
              <KeyRound :size="15" /> {{ active.hasKey ? '更新系统密钥' : '输入系统密钥' }}
            </button>
            <button v-if="active" class="icon-button danger" aria-label="删除模型档案" type="button" :disabled="pending" @click="remove"><Trash2 :size="16" /></button>
          </div>
          <p v-if="warning" class="form-error" role="alert">{{ warning }}</p>
          <div v-if="active" class="capabilities">
            <div class="caps-heading">
              <strong>接口能力</strong>
              <button class="flat-button" type="button" :disabled="pending || (active.needsKey && !active.hasKey)" @click="probe">
                <PlugZap :size="14" /> {{ pending ? '处理中' : '实测' }}
              </button>
            </div>
            <dl>
              <template v-for="(value, key) in labels" :key="key">
                <dt>{{ value }}</dt><dd>{{ states[active.capabilities[key] ?? 'unknown'] }}</dd>
              </template>
            </dl>
          </div>
        </form>
      </div>
    </section>
  </div>
</template>
