<script setup lang="ts">
import { computed, ref, toRaw } from 'vue'
import { BookMarked, CircleAlert, Search } from 'lucide-vue-next'
import AudienceFields from '../components/AudienceFields.vue'
import { errorMessage, activeTask } from '../core/contracts/types'
import { emptyAudience, eventTypes, type Canon, type Fact, type Knowledge, type Note, type SearchHit } from '../core/narrative/memory'
import { useStudio } from '../stores/studio'

const props = defineProps<{ reviewEventId?: string }>()
const emit = defineEmits<{ open: [chapter: string, quote: string, offset?: number] }>()
const studio = useStudio()
const tabs = { canon: '权威设定', review: '记忆审核', notes: '叙事备注', search: '正文检索', roles: '模型分工' } as const
const categories = { character: '人物', prop: '道具', location: '地点', world_rule: '世界规则' } as const
const statuses = { pending: '待确认', auto_accepted: '自动入库', author_accepted: '作者确认', rejected: '已撤回或拒绝' }
const tab = ref<keyof typeof tabs>('canon')
const characters = computed(() => studio.memory.canon.filter(entry => entry.category === 'character'))
const currentEvents = computed(() => studio.memory.events.filter(event => event.current))
const pendingFirst = computed(() => [...studio.memory.events].sort((a, b) =>
  Number(b.current && b.approval === 'pending') - Number(a.current && a.approval === 'pending')))
const revisionOptions = computed(() => studio.chapters.flatMap(ch => ch.currentRevisionId ? [{ id: ch.currentRevisionId, title: ch.title }] : []))
const createCanon = (): Canon => ({ entryId: '', version: 0, category: 'character', name: '', aliases: [], body: '', locked: true, audience: emptyAudience() })
const createNote = (): Note => ({ noteId: '', noteType: 'foreshadow', sourceRevisionId: studio.chapter?.currentRevisionId ?? '',
  relatedIds: [], body: '', audience: emptyAudience(), status: 'open' })
const canon = ref<Canon>(createCanon())
const aliases = ref('')
const note = ref<Note>(createNote())
const eventId = ref('')
const audience = ref(emptyAudience())
const fact = computed(() => studio.memory.events.find(event => event.eventId === eventId.value))
const knowledge = ref<Knowledge>()
const lookup = ref('')
const queried = ref('')
const hits = ref<SearchHit[]>([])
const searching = ref(false)
const searchCanon = computed(() => studio.memory.canon.filter(item => queried.value &&
  [item.name, ...item.aliases].some(name => name.normalize('NFKC').toLowerCase() === queried.value.normalize('NFKC').toLowerCase())))
const roles = ref({ ...studio.memory.roles })
const profileChoices = computed(() => studio.profiles.filter(item => !item.needsKey || item.hasKey))
const labels = { knows: '知道', suspects: '怀疑', misbelieves: '误信' }

function editCanon(entry?: Canon) {
  canon.value = entry ? structuredClone(toRaw(entry)) : createCanon()
  aliases.value = canon.value.aliases.join('、')
}
async function submitCanon() {
  canon.value.aliases = aliases.value.split(/[、，,]/).map(a => a.trim()).filter(Boolean)
  await studio.saveCanon(canon.value)
  if (!studio.error) editCanon()
}
function editEvent(event: Fact) {
  eventId.value = event.eventId
  audience.value = structuredClone(toRaw(event.audience))
  knowledge.value = undefined
}
function editNote(item: Note) { note.value = structuredClone(toRaw(item)) }
const initialEvent = studio.memory.events.find(event => event.eventId === props.reviewEventId)
if (initialEvent) {
  tab.value = 'review'
  editEvent(initialEvent)
}
function editKnowledge(event: Fact, old?: Knowledge) {
  knowledge.value = old ? { ...old } : { characterId: '', eventId: event.eventId, state: 'knows', source: 'observed',
    sinceRevisionId: event.sourceRevisionId, perception: '', observableClue: '' }
}
async function submitKnowledge() {
  if (!knowledge.value) return
  await studio.saveKnowledge(knowledge.value)
  if (!studio.error) knowledge.value = undefined
}
async function submitNote() {
  await studio.saveNote(note.value)
  if (!studio.error) note.value = createNote()
}
async function search() {
  queried.value = lookup.value.trim()
  if (!queried.value) { hits.value = []; return }
  searching.value = true
  try { hits.value = await studio.searchManuscript(queried.value) }
  catch (error) { studio.error = errorMessage(error) }
  finally { searching.value = false }
}
</script>

<template>
  <main class="workspace-page memory-page">
    <header class="workspace-intro"><h1><BookMarked :size="25" /> 记忆库</h1></header>
    <nav class="workspace-subnav" aria-label="记忆库导航">
      <button v-for="(label, id) in tabs" :key="id" type="button" class="flat-button" :class="{ selected: tab === id }"
        @click="tab = id">{{ label }} <span v-if="id === 'review' && studio.memoryPending" class="pending-count">{{ studio.memoryPending }}</span></button>
    </nav>
    <div v-if="tab === 'canon'" class="memory-grid">
      <section class="memory-list" aria-label="设定清单">
        <button type="button" class="secondary-button" @click="editCanon()">新建设定</button>
        <button v-for="entry in studio.memory.canon" :key="entry.entryId" type="button" class="memory-row"
          :class="{ selected: canon.entryId === entry.entryId }" @click="editCanon(entry)">
          <strong>{{ entry.name }}</strong><small>{{ categories[entry.category] }}{{ entry.locked ? ' · 作者锁定' : '' }}</small>
        </button>
        <p v-if="!studio.memory.canon.length" class="muted">先添加需要持续沿用的人物、道具、地点或世界规则。</p>
      </section>
      <form class="workspace-field memory-form" @submit.prevent="submitCanon">
        <h2>{{ canon.entryId ? '编辑设定' : '新建设定' }}</h2>
        <label>类型<select v-model="canon.category" :disabled="!!canon.entryId">
          <option v-for="(label, id) in categories" :key="id" :value="id">{{ label }}</option>
        </select></label>
        <label>名称<input v-model.trim="canon.name" maxlength="120" required></label>
        <label>别名（用顿号分隔）<input v-model="aliases"></label>
        <label>设定正文<textarea v-model="canon.body" rows="5"></textarea></label>
        <label class="check-line"><input v-model="canon.locked" type="checkbox"> 锁定为作者权威设定</label>
        <AudienceFields v-model="canon.audience" :characters="characters" />
        <button type="submit" class="primary-button" :disabled="studio.busy">保存设定</button>
      </form>
    </div>
    <div v-else-if="tab === 'review'" class="memory-grid">
      <section class="memory-list" aria-label="事件清单">
        <button v-for="event in pendingFirst" :key="event.eventId" type="button" class="memory-row"
          :class="{ selected: event.eventId === eventId }" @click="editEvent(event)">
          <strong>{{ eventTypes[event.eventType] }} · {{ event.summary }}</strong>
          <small :class="{ 'pending-text': event.current && event.approval === 'pending' }">第 {{ event.chapterOrder }} 章 · {{ statuses[event.approval] }}{{ event.current ? '' : ' · 旧修订' }}</small>
        </button>
        <p v-if="!studio.memory.events.length" class="muted">在正文点击「完成本章」即可启动独立审查和记忆提取。</p>
        <p v-for="[id, problem] in studio.memory.ingested.filter(pair => pair[1])" :key="id" class="pending-text">{{ problem }}</p>
      </section>
      <article v-if="fact" class="workspace-field memory-form">
        <h2>{{ eventTypes[fact.eventType] }}</h2>
        <p>{{ fact.summary }}</p>
        <p v-if="fact.time" class="muted">{{ fact.time }}</p>
        <blockquote>{{ fact.anchor.quote }}</blockquote>
        <button v-if="fact.current" type="button" class="flat-button" @click="emit('open', fact.chapterId, fact.anchor.quote, fact.anchor.startOffset)">在正文中定位</button>
        <template v-if="fact.current && fact.approval !== 'rejected'">
          <AudienceFields v-model="audience" :characters="characters" />
          <div class="memory-actions">
            <button type="button" class="primary-button" :disabled="studio.busy" @click="studio.reviewFact(fact, 'accept', audience)">
              {{ fact.approval === 'pending' ? '确认入库' : '更新知情范围' }}</button>
            <button v-if="fact.approval === 'pending'" type="button" class="secondary-button" :disabled="studio.busy" @click="studio.reviewFact(fact, 'reject')">拒绝</button>
            <button v-else type="button" class="secondary-button" :disabled="studio.busy" @click="studio.reviewFact(fact, 'undo')">撤回</button>
          </div>
          <template v-if="fact.approval !== 'pending'">
            <h3>人物知情</h3>
            <button v-for="row in studio.memory.knowledge.filter(item => item.eventId === fact?.eventId)" :key="row.characterId"
              type="button" class="memory-row" @click="editKnowledge(fact, row)">
              {{ characters.find(person => person.entryId === row.characterId)?.name }} · {{ labels[row.state] }}</button>
            <button type="button" class="secondary-button" :disabled="!characters.length" @click="editKnowledge(fact)">记录人物知情</button>
            <form v-if="knowledge" class="memory-form" @submit.prevent="submitKnowledge">
              <label>人物<select v-model="knowledge.characterId" required><option disabled value="">选择人物</option>
                <option v-for="person in characters" :key="person.entryId" :value="person.entryId">{{ person.name }}</option></select></label>
              <label>状态<select v-model="knowledge.state"><option v-for="(label, id) in labels" :key="id" :value="id">{{ label }}</option></select></label>
              <label>来源<select v-model="knowledge.source"><option value="observed">亲眼观察</option><option value="told">他人告知</option><option value="inferred">推断</option></select></label>
              <label>知情起点<select v-model="knowledge.sinceRevisionId" required>
                <option v-for="revision in revisionOptions" :key="revision.id" :value="revision.id">{{ revision.title }}</option>
              </select></label>
              <label v-if="knowledge.state !== 'knows'">人物以为发生了什么<input v-model="knowledge.perception" required maxlength="750"></label>
              <label>仅该人物可见的线索<input v-model="knowledge.observableClue" maxlength="600"></label>
              <button class="primary-button" type="submit" :disabled="studio.busy">保存人物知情</button>
            </form>
          </template>
        </template>
      </article>
    </div>
    <div v-else-if="tab === 'notes'" class="memory-grid">
      <section class="memory-list">
        <button class="secondary-button" type="button" @click="note = createNote()">新建备注</button>
        <button v-for="item in studio.memory.notes" :key="item.noteId" type="button" class="memory-row"
          @click="editNote(item)">{{ item.body }}</button>
      </section>
      <form class="workspace-field memory-form" @submit.prevent="submitNote">
        <h2>叙事备注</h2>
        <label>类型<select v-model="note.noteType"><option value="foreshadow">伏笔</option><option value="gag_hook">包袱回旋</option><option value="subtext_cue">潜台词</option></select></label>
        <label>来源章节<select v-model="note.sourceRevisionId" required><option value="" disabled>选择已保存的章节</option>
          <option v-if="note.sourceRevisionId && !revisionOptions.some(item => item.id === note.sourceRevisionId)" :value="note.sourceRevisionId">保留原始修订</option>
          <option v-for="revision in revisionOptions" :key="revision.id" :value="revision.id">{{ revision.title }}</option></select></label>
        <label>进度<select v-model="note.status"><option value="open">待回收</option><option value="reminded">已提醒</option><option value="paid_off">已回收</option><option value="deprecated">已废弃</option></select></label>
        <label>关联事实与设定<select v-model="note.relatedIds" multiple>
          <option v-for="entry in studio.memory.canon" :key="entry.entryId" :value="entry.entryId">{{ entry.name }}</option>
          <option v-for="event in currentEvents.filter(item => ['auto_accepted','author_accepted'].includes(item.approval))" :key="event.eventId" :value="event.eventId">{{ event.summary }}</option>
        </select></label>
        <label>备注<textarea v-model="note.body" rows="4" required></textarea></label>
        <AudienceFields v-model="note.audience" :characters="characters" />
        <button type="submit" class="primary-button" :disabled="studio.busy">保存备注</button>
      </form>
    </div>
    <section v-else-if="tab === 'search'" class="workspace-field memory-form memory-search">
      <form class="memory-actions" @submit.prevent="search">
        <input v-model="lookup" aria-label="检索正文或设定" placeholder="输入设定名称、别名或连续汉字" required>
        <button type="submit" class="primary-button" :disabled="searching"><Search :size="15" /> 查找</button>
      </form>
      <button v-for="entry in searchCanon" :key="entry.entryId" type="button" class="memory-row" @click="editCanon(entry); tab = 'canon'">
        <strong>{{ entry.name }}</strong><small>权威设定 · {{ categories[entry.category] }}</small>
      </button>
      <button v-for="hit in hits" :key="hit.revisionId" type="button" class="memory-row" @click="emit('open', hit.chapterId, queried)">
        <strong>第 {{ hit.chapterOrder }} 章 · {{ hit.title }}</strong><small>{{ hit.excerpt }}</small></button>
      <p v-if="queried && !searching && !hits.length && !searchCanon.length" class="muted">没有匹配项。正文检索需要连续的两个或以上汉字。</p>
    </section>
    <form v-else class="workspace-field memory-form memory-search" @submit.prevent="studio.saveRoles(roles)">
      <h2>审查与记忆提取</h2>
      <p class="muted">配置两个角色后，采纳的正文达到章节字数目标时会自动分别提交审查与提取。审查看到作者全知设定。</p>
      <label>一致性审查<select v-model="roles.auditProfileId"><option :value="null">手动选用主笔模型</option>
        <option v-for="profile in profileChoices" :key="profile.id" :value="profile.id">{{ profile.name }}</option></select></label>
      <label>证据提取<select v-model="roles.extractionProfileId"><option :value="null">手动选用主笔模型</option>
        <option v-for="profile in profileChoices" :key="profile.id" :value="profile.id">{{ profile.name }}</option></select></label>
      <button type="submit" class="primary-button" :disabled="studio.busy">保存分工</button>
      <p v-if="studio.chapterAudits.some(task => activeTask(task.state))" class="muted">本章审查进行中</p>
      <p v-for="task in studio.chapterAudits.filter(item => studio.writingIssues[item.taskId])" :key="task.taskId" class="pending-text">
        <CircleAlert :size="14" /> {{ studio.writingIssues[task.taskId] }}</p>
    </form>
  </main>
</template>
