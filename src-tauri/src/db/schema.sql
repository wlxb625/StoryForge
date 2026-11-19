PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA foreign_keys = ON;

-- 1. 工程元数据与版本迁移
CREATE TABLE IF NOT EXISTS project_meta (
    project_id TEXT PRIMARY KEY,
    schema_version INTEGER NOT NULL DEFAULT 1,
    title TEXT NOT NULL,
    active_pack_id TEXT,
    active_base_skill_id TEXT,
    target_beat_chars INTEGER DEFAULT 900,
    target_chapter_chars INTEGER DEFAULT 2000,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 2. 粗线条阶段目标与自适应改纲
CREATE TABLE IF NOT EXISTS stage_arcs (
    arc_id TEXT PRIMARY KEY,
    arc_order INTEGER NOT NULL,
    title TEXT NOT NULL,
    rough_goal_text TEXT NOT NULL,
    expected_chapters INTEGER DEFAULT 12,
    milestones_json TEXT DEFAULT '[]',
    pivot_instruction TEXT DEFAULT '',
    pivot_remaining_chapters INTEGER DEFAULT 0, -- 0~N 自适应过渡
    status TEXT DEFAULT 'active',
    updated_at TEXT NOT NULL
);

-- 3. 章节逻辑表（不可变 UUID 主键，支持任意插删调序）
CREATE TABLE IF NOT EXISTS chapters (
    chapter_id TEXT PRIMARY KEY,
    arc_id TEXT REFERENCES stage_arcs(arc_id),
    chapter_order INTEGER NOT NULL,
    thread_id TEXT DEFAULT 'main',
    title TEXT NOT NULL DEFAULT '',
    current_revision_id TEXT,
    memory_sync_status TEXT DEFAULT 'pending', -- pending/processing/synced/needs_confirm
    memory_version INTEGER DEFAULT 0,
    status TEXT DEFAULT 'planned',             -- planned/draft/accepted/published
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 4. 正文不可变修订版表（区分临时缓冲/AI草稿/正式修订）
CREATE TABLE IF NOT EXISTS chapter_revisions (
    revision_id TEXT PRIMARY KEY,
    chapter_id TEXT NOT NULL REFERENCES chapters(chapter_id),
    parent_revision_id TEXT REFERENCES chapter_revisions(revision_id),
    save_kind TEXT NOT NULL,                   -- auto_buffer / ai_draft / formal_revision
    edit_level TEXT DEFAULT 'semantic',        -- surface / semantic / canon
    source_tag TEXT NOT NULL,                  -- ai_gen / manual_edit / polish_diff / interrupted_draft
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    cjk_char_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

-- 5. 作者权威设定库（Canon，支持锁定防覆盖）
CREATE TABLE IF NOT EXISTS canon_entries (
    entry_id TEXT PRIMARY KEY,
    category TEXT NOT NULL,                    -- character / prop / location / world_rule
    name TEXT NOT NULL,
    aliases_json TEXT DEFAULT '[]',            -- 别名与曾用名，供实体精确检索
    attributes_json TEXT DEFAULT '{}',         -- 前端单向渲染为 Markdown 卡，编辑直接回写
    is_locked INTEGER DEFAULT 1,               -- 1=作者锁定，AI 严禁静默覆盖
    version INTEGER DEFAULT 1,
    updated_at TEXT NOT NULL
);

-- 6. 已确认/候选事件记录（带原文证据锚点与置信度）
CREATE TABLE IF NOT EXISTS story_events (
    event_id TEXT PRIMARY KEY,
    source_revision_id TEXT NOT NULL REFERENCES chapter_revisions(revision_id),
    event_type TEXT NOT NULL,                  -- prop_transfer/location_move/secret_created/relation_shift/plot_event
    subject_ids_json TEXT DEFAULT '[]',
    summary TEXT NOT NULL,
    payload_json TEXT DEFAULT '{}',
    evidence_anchor_json TEXT NOT NULL,        -- {quote, start_offset, end_offset}
    confidence TEXT NOT NULL,                  -- high / medium / low
    approval_status TEXT NOT NULL,             -- pending / auto_accepted / author_accepted / rejected
    world_time_label TEXT DEFAULT '',
    created_at TEXT NOT NULL
);

-- 7. 人物知情状态表（支撑 POV 物理隔离）
CREATE TABLE IF NOT EXISTS character_knowledge (
    knowledge_id TEXT PRIMARY KEY,
    character_id TEXT NOT NULL REFERENCES canon_entries(entry_id),
    target_event_id TEXT NOT NULL REFERENCES story_events(event_id),
    epistemic_state TEXT NOT NULL,             -- knows / suspects / misbelieves
    knowledge_source TEXT NOT NULL,            -- observed / told / inferred
    known_since_revision_id TEXT NOT NULL REFERENCES chapter_revisions(revision_id),
    is_secret INTEGER DEFAULT 0,               -- 1=秘密本体，对未上榜 POV 物理屏蔽
    observable_clues TEXT DEFAULT '',          -- 仅投喂给不知情 POV 的表面反常线索
    updated_at TEXT NOT NULL
);

-- 8. 伏笔、包袱回旋镖与软语义备注
CREATE TABLE IF NOT EXISTS soft_notes (
    note_id TEXT PRIMARY KEY,
    note_type TEXT NOT NULL,                   -- foreshadow / gag_hook / subtext_cue
    related_ids_json TEXT DEFAULT '[]',
    content TEXT NOT NULL,
    audience_scope TEXT DEFAULT 'public',      -- public / named_characters / narrator_only
    visible_to_json TEXT DEFAULT '[]',         -- POV 过滤白名单
    source_revision_id TEXT NOT NULL REFERENCES chapter_revisions(revision_id),
    status TEXT DEFAULT 'open',                -- open / paid_off / reminded / deprecated
    created_at TEXT NOT NULL
);

-- 9. 流派卡带与 Writing Skills 存储表（纯声明式，无可执行代码）
CREATE TABLE IF NOT EXISTS style_packs (
    item_id TEXT PRIMARY KEY,
    item_kind TEXT NOT NULL,                   -- genre_cartridge / writing_skill
    slot TEXT DEFAULT 'technique',             -- base_style / technique / cartridge
    name TEXT NOT NULL,
    version TEXT NOT NULL,
    priority INTEGER DEFAULT 20,
    metadata_json TEXT NOT NULL,               -- triggers / anti_triggers / avoid_phrases
    body_markdown TEXT NOT NULL,               -- 核心技法 + 正反例样章
    is_enabled INTEGER DEFAULT 1,
    updated_at TEXT NOT NULL
);

-- 10. 持久化生成任务表（任务与订阅解耦 + 冻结上下文快照）
CREATE TABLE IF NOT EXISTS generation_tasks (
    task_id TEXT PRIMARY KEY,
    idempotency_key TEXT NOT NULL UNIQUE,
    request_hash TEXT NOT NULL,
    chapter_id TEXT NOT NULL REFERENCES chapters(chapter_id),
    base_revision_id TEXT,
    task_type TEXT NOT NULL,                   -- beat_write/polish_span/consistency_check/memory_extract/critic
    state TEXT NOT NULL,                       -- created/queued/running/streaming/completed/failed/cancelled/interrupted/outcome_unknown
    frozen_context_json TEXT NOT NULL,         -- 冻结的设定版本/意图/Skill版本/实际Prompt
    partial_output TEXT DEFAULT '',            -- 断流保全草稿
    last_event_seq INTEGER DEFAULT 0,
    provider_request_id TEXT DEFAULT '',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

-- 11. 任务事件流持久化表（支持断线按 event_seq 重放）
CREATE TABLE IF NOT EXISTS task_events (
    task_id TEXT NOT NULL REFERENCES generation_tasks(task_id),
    event_seq INTEGER NOT NULL,
    event_type TEXT NOT NULL,                  -- status/token/gate_warning/completed/error/outcome_unknown
    payload_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (task_id, event_seq)
);

-- 12. 全链路模型调用留痕表（脱敏，记录真实 usage）
CREATE TABLE IF NOT EXISTS llm_traces (
    trace_id TEXT PRIMARY KEY,
    task_id TEXT REFERENCES generation_tasks(task_id),
    model_profile_id TEXT NOT NULL,
    stage TEXT NOT NULL,
    prompt_tokens INTEGER DEFAULT 0,
    cached_tokens INTEGER,                     -- NULL 表示供应商未返回，严禁伪造为 0
    completion_tokens INTEGER DEFAULT 0,
    cost_usd REAL,
    latency_ms INTEGER DEFAULT 0,
    status TEXT NOT NULL,
    created_at TEXT NOT NULL
);

-- 13. 中文 Bigram 全文检索无内容虚拟表（仅存倒排索引，不重复存原文）
CREATE VIRTUAL TABLE IF NOT EXISTS chapters_fts USING fts5(
    chapter_id UNINDEXED,
    bigram_text,
    content='',
    tokenize='unicode61'
);

CREATE INDEX IF NOT EXISTS idx_revisions_chapter ON chapter_revisions(chapter_id, created_at);
CREATE INDEX IF NOT EXISTS idx_events_rev ON story_events(source_revision_id, approval_status);
CREATE INDEX IF NOT EXISTS idx_knowledge_char ON character_knowledge(character_id, is_secret);
CREATE INDEX IF NOT EXISTS idx_tasks_state ON generation_tasks(state, updated_at);
