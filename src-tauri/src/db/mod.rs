use std::{
    fs::OpenOptions,
    path::{Path, PathBuf},
    time::Duration,
};

use chrono::{SecondsFormat, Utc};
use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Row, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::{Result, StudioError};

pub mod catalog;
pub mod memory;

pub const SCHEMA: &str = include_str!("schema.sql");
const TASK_COLUMNS: &str = "task_id, chapter_id, base_revision_id, state, frozen_context_json, partial_output, last_event_seq, task_type";

pub fn now() -> String {
    Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn id() -> String {
    Uuid::new_v4().to_string()
}

fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}

pub fn han_count(content: &str) -> usize {
    content.chars().filter(|&ch| matches!(ch as u32, 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F | 0x30000..=0x323AF)).count()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    pub project_id: String,
    pub title: String,
    pub path: String,
    pub target_beat_chars: i64,
    pub target_chapter_chars: i64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Chapter {
    pub chapter_id: String,
    pub title: String,
    pub chapter_order: i64,
    pub current_revision_id: Option<String>,
    pub status: String,
    pub active_scene_id: Option<String>,
    pub memory_sync_status: String,
}

fn read_chapter(row: &Row<'_>) -> rusqlite::Result<Chapter> {
    Ok(Chapter {
        chapter_id: row.get(0)?,
        title: row.get(1)?,
        chapter_order: row.get(2)?,
        current_revision_id: row.get(3)?,
        status: row.get(4)?,
        active_scene_id: row.get(5)?,
        memory_sync_status: row.get(6)?,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Revision {
    pub revision_id: String,
    pub parent_revision_id: Option<String>,
    pub save_kind: String,
    pub source_tag: String,
    pub content: String,
    pub cjk_char_count: i64,
    pub created_at: String,
}

fn read_revision(row: &Row<'_>) -> rusqlite::Result<Revision> {
    Ok(Revision {
        revision_id: row.get(0)?,
        parent_revision_id: row.get(1)?,
        save_kind: row.get(2)?,
        source_tag: row.get(3)?,
        content: row.get(4)?,
        cjk_char_count: row.get(5)?,
        created_at: row.get(6)?,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionInput {
    pub chapter_id: String,
    pub base_revision_id: Option<String>,
    pub content: String,
    pub source: String,
    pub draft_task_id: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub task_id: String,
    pub chapter_id: String,
    pub base_revision_id: Option<String>,
    pub state: String,
    pub frozen_context: Value,
    pub partial_output: String,
    pub last_event_seq: i64,
    pub task_type: String,
}

fn read_task(row: &Row<'_>) -> rusqlite::Result<Task> {
    let snapshot: String = row.get(4)?;
    let frozen_context = serde_json::from_str(&snapshot).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(Task {
        task_id: row.get(0)?,
        chapter_id: row.get(1)?,
        base_revision_id: row.get(2)?,
        state: row.get(3)?,
        frozen_context,
        partial_output: row.get(5)?,
        last_event_seq: row.get(6)?,
        task_type: row.get(7)?,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInput {
    pub idempotency_key: String,
    pub chapter_id: String,
    pub base_revision_id: Option<String>,
    pub task_type: String,
    pub frozen_context: Value,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskEvent {
    pub task_id: String,
    pub event_seq: i64,
    pub event_type: String,
    pub payload: Value,
}

#[derive(Default)]
pub struct Metering {
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub latency_ms: i64,
    pub profile_id: String,
}

pub struct Project {
    conn: Connection,
    pub info: ProjectInfo,
}

fn validate_path(path: &Path) -> Result<()> {
    if !path
        .extension()
        .and_then(|value| value.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("story"))
    {
        return Err(StudioError::new(400, "请选择 .story 小说工程文件"));
    }
    Ok(())
}

impl Project {
    pub fn create(path: PathBuf, title: &str) -> Result<Self> {
        validate_path(&path)?;
        if title.trim().is_empty() || title.chars().count() > 120 {
            return Err(StudioError::new(400, "请填写 1 至 120 字的书名"));
        }
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::AlreadyExists {
                    StudioError::new(409, "这个工程文件已经存在，请打开它或选择新文件名")
                } else {
                    err.into()
                }
            })?;
        let mut conn = Connection::open(&path)?;
        conn.execute_batch(SCHEMA)?;
        let tx = conn.transaction()?;
        let current = now();
        tx.execute(
            "INSERT INTO project_meta(project_id,title,created_at,updated_at) VALUES (?1,?2,?3,?3)",
            params![id(), title.trim(), current],
        )?;
        tx.execute("INSERT INTO chapters(chapter_id,chapter_order,title,created_at,updated_at) VALUES (?1,1,?2,?3,?3)",
            params![id(), "未命名章节", current])?;
        tx.commit()?;
        Self::from_connection(conn, path)
    }

    pub fn open(path: PathBuf) -> Result<Self> {
        validate_path(&path)?;
        let conn = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_WRITE)?;
        Self::from_connection(conn, path)
    }

    fn from_connection(mut conn: Connection, path: PathBuf) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(5))?;
        let version: i64 = conn
            .query_row(
                "SELECT schema_version FROM project_meta LIMIT 1",
                [],
                |row| row.get(0),
            )
            .map_err(|_| StudioError::new(400, "这个文件不是有效的 StoryForge 工程"))?;
        if !(1..=3).contains(&version) {
            return Err(StudioError::new(409, "工程版本与当前应用不兼容"));
        }
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA foreign_keys = ON;",
        )?;
        if version < 3 {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let current: i64 =
                tx.query_row("SELECT schema_version FROM project_meta LIMIT 1", [], |r| {
                    r.get(0)
                })?;
            if current == 1 {
                tx.execute_batch(include_str!("002_creation.sql"))?;
                tx.execute("UPDATE project_meta SET schema_version=2", [])?;
            }
            if current < 3 {
                tx.execute_batch(include_str!("003_memory.sql"))?;
                tx.execute("UPDATE project_meta SET schema_version=3", [])?;
            }
            tx.commit()?;
        }
        let info = conn.query_row("SELECT project_id,title,target_beat_chars,target_chapter_chars FROM project_meta LIMIT 1", [], |r| {
            Ok(ProjectInfo { project_id: r.get(0)?, title: r.get(1)?, path: path.to_string_lossy().into(),
                target_beat_chars: r.get(2)?, target_chapter_chars: r.get(3)? })
        })?;
        Ok(Self { conn, info })
    }

    pub fn chapters(&self) -> Result<Vec<Chapter>> {
        let mut query = self.conn.prepare(
            "SELECT chapter_id,title,chapter_order,current_revision_id,status,active_scene_id,memory_sync_status FROM chapters
            WHERE status!='archived' ORDER BY chapter_order, chapter_id",
        )?;
        let rows = query
            .query_map([], read_chapter)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn archived_chapters(&self) -> Result<Vec<Chapter>> {
        let mut query = self.conn.prepare(
            "SELECT chapter_id,title,chapter_order,current_revision_id,status,active_scene_id,memory_sync_status FROM chapters
            WHERE status='archived' ORDER BY updated_at DESC",
        )?;
        let rows = query
            .query_map([], read_chapter)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn restore_chapter(&self, chapter_id: &str) -> Result<()> {
        let changed = self.conn.execute("UPDATE chapters SET
            chapter_order=(SELECT COALESCE(MAX(chapter_order),0)+1 FROM chapters WHERE status!='archived'),
            status=CASE WHEN current_revision_id IS NULL THEN 'planned' ELSE 'accepted' END, updated_at=?1
            WHERE chapter_id=?2 AND status='archived'", params![now(),chapter_id])?;
        if changed == 0 {
            return Err(StudioError::new(404, "没有找到归档章节"));
        }
        Ok(())
    }

    pub fn rename_project(&mut self, title: &str) -> Result<ProjectInfo> {
        let title = title.trim();
        if title.is_empty() || title.chars().count() > 120 {
            return Err(StudioError::new(400, "请填写 1 至 120 字的书名"));
        }
        self.conn.execute(
            "UPDATE project_meta SET title=?1,updated_at=?2",
            params![title, now()],
        )?;
        self.info.title = title.into();
        Ok(self.info.clone())
    }

    pub fn insert_chapter(&mut self, title: &str, after: Option<&str>) -> Result<Chapter> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let position: i64 = if let Some(previous) = after {
            tx.query_row(
                "SELECT chapter_order+1 FROM chapters WHERE chapter_id=?1 AND status!='archived'",
                [previous],
                |row| row.get(0),
            )?
        } else {
            tx.query_row(
                "SELECT COALESCE(MAX(chapter_order),0)+1 FROM chapters WHERE status!='archived'",
                [],
                |row| row.get(0),
            )?
        };
        tx.execute("UPDATE chapters SET chapter_order=chapter_order+1 WHERE chapter_order>=?1 AND status!='archived'", [position])?;
        let chapter_id = id();
        let title = chapter_title(title)?;
        tx.execute("INSERT INTO chapters(chapter_id,chapter_order,title,created_at,updated_at) VALUES (?1,?2,?3,?4,?4)",
            params![chapter_id, position, title, now()])?;
        tx.commit()?;
        Ok(Chapter {
            chapter_id,
            title,
            chapter_order: position,
            current_revision_id: None,
            status: "planned".into(),
            active_scene_id: None,
            memory_sync_status: "pending".into(),
        })
    }

    pub fn rename_chapter(&self, chapter_id: &str, title: &str) -> Result<()> {
        let changed = self.conn.execute(
            "UPDATE chapters SET title=?1,updated_at=?2 WHERE chapter_id=?3 AND status!='archived'",
            params![chapter_title(title)?, now(), chapter_id],
        )?;
        if changed == 0 {
            return Err(StudioError::new(404, "没有找到章节"));
        }
        Ok(())
    }

    pub fn move_chapter(&mut self, chapter_id: &str, direction: i64) -> Result<()> {
        if !matches!(direction, -1 | 1) {
            return Err(StudioError::new(400, "无效的移动方向"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let order: i64 = tx.query_row(
            "SELECT chapter_order FROM chapters WHERE chapter_id=?1 AND status!='archived'",
            [chapter_id],
            |row| row.get(0),
        )?;
        let neighbor = tx
            .query_row(
                "SELECT chapter_id FROM chapters WHERE chapter_order=?1 AND status!='archived'",
                [order + direction],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(neighbor) = neighbor {
            tx.execute(
                "UPDATE chapters SET chapter_order=?1, updated_at=?2 WHERE chapter_id=?3",
                params![order + direction, now(), chapter_id],
            )?;
            tx.execute(
                "UPDATE chapters SET chapter_order=?1, updated_at=?2 WHERE chapter_id=?3",
                params![order, now(), neighbor],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn archive_chapter(&mut self, chapter_id: &str) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let running: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM generation_tasks WHERE chapter_id=?1 AND
            state IN ('queued','running','streaming'))",
            [chapter_id],
            |row| row.get(0),
        )?;
        if running {
            return Err(StudioError::new(409, "先结束该章正在运行的任务"));
        }
        let order: i64 = tx.query_row(
            "SELECT chapter_order FROM chapters WHERE chapter_id=?1 AND status!='archived'",
            [chapter_id],
            |row| row.get(0),
        )?;
        tx.execute(
            "UPDATE chapters SET status='archived', updated_at=?1 WHERE chapter_id=?2",
            params![now(), chapter_id],
        )?;
        tx.execute("UPDATE chapters SET chapter_order=chapter_order-1 WHERE status!='archived' AND chapter_order>?1", [order])?;
        tx.commit()?;
        Ok(())
    }

    pub fn revisions(&self, chapter_id: &str) -> Result<Vec<Revision>> {
        let mut query = self.conn.prepare("SELECT revision_id,parent_revision_id,save_kind,source_tag,content,cjk_char_count,created_at
            FROM chapter_revisions WHERE chapter_id=?1 ORDER BY rowid DESC")?;
        let rows = query
            .query_map([chapter_id], read_revision)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn save_revision(&mut self, input: RevisionInput) -> Result<Revision> {
        if input.content.len() > 8_000_000 {
            return Err(StudioError::new(413, "单次保存的正文太长"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let base = current_revision(&tx, &input.chapter_id)?;
        if base != input.base_revision_id {
            return Err(StudioError::new(
                409,
                "章节已有更新，请查看最新修订后再保存",
            ));
        }
        let (save_kind, source_tag, content) = match input.source.as_str() {
            "manual_edit" => ("formal_revision", "manual_edit", input.content),
            "auto_buffer" => {
                let prior = tx.query_row("SELECT revision_id,parent_revision_id,save_kind,source_tag,content,cjk_char_count,created_at
                    FROM chapter_revisions WHERE chapter_id=?1 AND parent_revision_id IS ?2 AND save_kind='auto_buffer' ORDER BY rowid DESC LIMIT 1",
                    params![input.chapter_id, base], read_revision).optional()?;
                if let Some(prior) = prior.filter(|prior| prior.content == input.content) {
                    return Ok(prior);
                }
                ("auto_buffer", "manual_edit", input.content)
            }
            "ai_gen" | "interrupted_draft" => {
                let task_id = input
                    .draft_task_id
                    .as_deref()
                    .ok_or_else(|| StudioError::new(400, "缺少生成任务"))?;
                let task = find_task(&tx, task_id)?;
                if !matches!(task.task_type.as_str(), "beat_write" | "ab_write")
                    || task.chapter_id != input.chapter_id
                    || task.base_revision_id != base
                {
                    return Err(StudioError::new(
                        409,
                        "这份草稿的基线已过期，仍可从任务历史复制正文",
                    ));
                }
                if task.partial_output.trim().is_empty()
                    || (input.source == "ai_gen" && task.state != "completed")
                    || matches!(task.state.as_str(), "queued" | "running" | "streaming")
                {
                    return Err(StudioError::new(409, "该任务暂时没有可保存的草稿"));
                }
                if input.source == "interrupted_draft" {
                    let carry = task.frozen_context["carryDraft"]
                        .as_str()
                        .unwrap_or_default();
                    (
                        "ai_draft",
                        "interrupted_draft",
                        append_drafts(String::new(), [carry, task.partial_output.as_str()]),
                    )
                } else {
                    let prior: String = match &base {
                        Some(revision) => tx.query_row(
                            "SELECT content FROM chapter_revisions WHERE revision_id=?1",
                            [revision],
                            |r| r.get(0),
                        )?,
                        None => String::new(),
                    };
                    let carry = task.frozen_context["carryDraft"]
                        .as_str()
                        .unwrap_or_default();
                    let joined = append_drafts(prior, [carry, task.partial_output.as_str()]);
                    ("formal_revision", "ai_gen", joined)
                }
            }
            _ => return Err(StudioError::new(400, "无法识别保存来源")),
        };
        if content.len() > 8_000_000 {
            return Err(StudioError::new(413, "章节正文超出保存上限"));
        }
        let revision = Revision {
            revision_id: id(),
            parent_revision_id: base,
            save_kind: save_kind.into(),
            source_tag: source_tag.into(),
            cjk_char_count: han_count(&content) as i64,
            content,
            created_at: now(),
        };
        tx.execute("INSERT INTO chapter_revisions(revision_id,chapter_id,parent_revision_id,save_kind,source_tag,content,content_hash,cjk_char_count,created_at)
            VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![revision.revision_id, input.chapter_id, revision.parent_revision_id, revision.save_kind, revision.source_tag,
                revision.content, hash(&revision.content), revision.cjk_char_count, revision.created_at])?;
        if save_kind == "formal_revision" {
            if source_tag == "ai_gen" {
                tx.execute("UPDATE story_events SET active_revision_id=?1 WHERE active_revision_id=?2 AND event_type!='chapter_summary'",
                    params![revision.revision_id,revision.parent_revision_id])?;
            }
            tx.execute("UPDATE chapters SET current_revision_id=?1,status='accepted',memory_sync_status='pending',updated_at=?2 WHERE chapter_id=?3",
                params![revision.revision_id, revision.created_at, input.chapter_id])?;
        }
        tx.commit()?;
        Ok(revision)
    }

    pub fn create_task(&mut self, input: TaskInput) -> Result<Task> {
        if input.idempotency_key.is_empty()
            || input.idempotency_key.len() > 128
            || !matches!(
                input.task_type.as_str(),
                "beat_write"
                    | "ab_write"
                    | "skill_extract"
                    | "memory_extract"
                    | "consistency_check"
            )
        {
            return Err(StudioError::new(400, "无效的节拍生成请求"));
        }
        let frozen = serde_json::to_string(&input.frozen_context)?;
        if frozen.len() > 2_000_000
            || input.frozen_context["prompt"]
                .as_str()
                .is_none_or(str::is_empty)
        {
            return Err(StudioError::new(400, "生成提示词为空或超过长度限制"));
        }
        let hash = hash(&serde_json::to_string(&json!([
            input.chapter_id,
            input.base_revision_id,
            input.task_type,
            input.frozen_context
        ]))?);
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let duplicate = tx
            .query_row(
                "SELECT task_id,request_hash FROM generation_tasks WHERE idempotency_key=?1",
                [&input.idempotency_key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        if let Some((existing_id, existing_hash)) = duplicate {
            if existing_hash != hash {
                return Err(StudioError::new(409, "这个请求标识已用于不同的生成内容"));
            }
            return find_task(&tx, &existing_id);
        }
        if current_revision(&tx, &input.chapter_id)? != input.base_revision_id {
            return Err(StudioError::new(409, "请刷新章节后再续写"));
        }
        let task_id = id();
        tx.execute("INSERT INTO generation_tasks(task_id,idempotency_key,request_hash,chapter_id,base_revision_id,task_type,state,frozen_context_json,created_at,updated_at)
            VALUES (?1,?2,?3,?4,?5,?6,'queued',?7,?8,?8)",
            params![task_id, input.idempotency_key, hash,input.chapter_id,input.base_revision_id,input.task_type,frozen,now()])?;
        append_event(
            &tx,
            &task_id,
            "status",
            json!({"state":"queued"}),
            Some("queued"),
            "",
        )?;
        let task = find_task(&tx, &task_id)?;
        tx.commit()?;
        Ok(task)
    }

    pub fn task(&self, id: &str) -> Result<Task> {
        find_task(&self.conn, id)
    }

    pub fn tasks(&self, chapter_id: Option<&str>) -> Result<Vec<Task>> {
        let mut query = self.conn.prepare(&format!(
            "SELECT {TASK_COLUMNS} FROM generation_tasks WHERE (?1 IS NULL OR chapter_id=?1)
            ORDER BY rowid DESC"
        ))?;
        let rows = query
            .query_map([chapter_id], read_task)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn events(&self, task_id: &str, from: i64) -> Result<Vec<TaskEvent>> {
        let mut query = self.conn.prepare(
            "SELECT event_seq,event_type,payload_json FROM task_events
            WHERE task_id=?1 AND event_seq>?2 ORDER BY event_seq LIMIT 1000",
        )?;
        let entries = query.query_map(params![task_id, from], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get::<_, String>(2)?))
        })?;
        entries
            .map(|entry| {
                let (event_seq, event_type, payload) = entry?;
                Ok(TaskEvent {
                    task_id: task_id.to_owned(),
                    event_seq,
                    event_type,
                    payload: serde_json::from_str(&payload)?,
                })
            })
            .collect()
    }

    pub fn claim_task(&mut self, task_id: &str) -> Result<Option<TaskEvent>> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if find_task(&tx, task_id)?.state != "queued" {
            return Ok(None);
        }
        let event = append_event(
            &tx,
            task_id,
            "status",
            json!({"state":"running"}),
            Some("running"),
            "",
        )?;
        tx.commit()?;
        Ok(Some(event))
    }

    pub fn append_token(&mut self, task_id: &str, text: &str) -> Result<Option<TaskEvent>> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if !matches!(state_of(&tx, task_id)?.as_str(), "running" | "streaming") {
            return Ok(None);
        }
        let event = append_event(
            &tx,
            task_id,
            "token",
            json!({"state":"streaming","text":text}),
            Some("streaming"),
            text,
        )?;
        tx.commit()?;
        Ok(Some(event))
    }

    pub fn finish_task(
        &mut self,
        task_id: &str,
        state: &str,
        message: Option<&str>,
        finish_reason: Option<&str>,
        usage: Option<Metering>,
    ) -> Result<Option<TaskEvent>> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let previous = state_of(&tx, task_id)?;
        if !matches!(previous.as_str(), "queued" | "running" | "streaming")
            && !(state == "failed" && previous == "completed" && message.is_some())
        {
            return Ok(None);
        }
        let event_type = match state {
            "completed" => "completed",
            "outcome_unknown" => "outcome_unknown",
            "failed" => "error",
            _ => "status",
        };
        let event = append_event(
            &tx,
            task_id,
            event_type,
            json!({"state":state,"message":message,"finishReason":finish_reason}),
            Some(state),
            "",
        )?;
        if let Some(usage) = usage {
            tx.execute("INSERT INTO llm_traces(trace_id,task_id,model_profile_id,stage,prompt_tokens,cached_tokens,completion_tokens,cost_usd,latency_ms,status,created_at)
                VALUES (?1,?2,?3,(SELECT task_type FROM generation_tasks WHERE task_id=?2),?4,?5,?6,?7,?8,?9,?10)",
                params![id(), task_id, usage.profile_id, usage.prompt_tokens, usage.cached_tokens, usage.completion_tokens, usage.cost_usd, usage.latency_ms, state, now()])?;
        }
        tx.commit()?;
        Ok(Some(event))
    }

    pub fn recover(&mut self) -> Result<Vec<Task>> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let interrupted = {
            let mut query = tx.prepare(
                "SELECT task_id FROM generation_tasks WHERE state IN ('running','streaming')",
            )?;
            let ids = query
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            ids
        };
        let mut rows = Vec::new();
        for task_id in interrupted {
            append_event(
                &tx,
                &task_id,
                "outcome_unknown",
                json!({"state":"outcome_unknown","message":"应用在请求发出后中断，远端结果未知。"}),
                Some("outcome_unknown"),
                "",
            )?;
            rows.push(find_task(&tx, &task_id)?);
        }
        tx.commit()?;
        Ok(rows)
    }
}

fn append_drafts(mut content: String, parts: [&str; 2]) -> String {
    for part in parts
        .map(str::trim_start)
        .into_iter()
        .filter(|part| !part.trim().is_empty())
    {
        if !content.is_empty() {
            if !content.ends_with('\n') {
                content.push('\n');
            }
            if !content.ends_with("\n\n") {
                content.push('\n');
            }
        }
        content.push_str(part);
    }
    content
}

fn chapter_title(title: &str) -> Result<String> {
    let title = title.trim();
    if title.chars().count() > 120 {
        return Err(StudioError::new(400, "章节名不能超过 120 字"));
    }
    Ok(if title.is_empty() {
        "未命名章节".into()
    } else {
        title.into()
    })
}

fn current_revision(conn: &Connection, chapter_id: &str) -> Result<Option<String>> {
    Ok(conn.query_row(
        "SELECT current_revision_id FROM chapters WHERE chapter_id=?1 AND status!='archived'",
        [chapter_id],
        |r| r.get(0),
    )?)
}

fn find_task(conn: &Connection, task_id: &str) -> Result<Task> {
    Ok(conn.query_row(
        &format!("SELECT {TASK_COLUMNS} FROM generation_tasks WHERE task_id=?1"),
        [task_id],
        read_task,
    )?)
}

fn state_of(conn: &Connection, task_id: &str) -> Result<String> {
    Ok(conn.query_row(
        "SELECT state FROM generation_tasks WHERE task_id=?1",
        [task_id],
        |r| r.get(0),
    )?)
}

fn append_event(
    tx: &Transaction<'_>,
    task_id: &str,
    event_type: &str,
    payload: Value,
    state: Option<&str>,
    token: &str,
) -> Result<TaskEvent> {
    let seq = tx.query_row("UPDATE generation_tasks SET partial_output=partial_output||?1,
        last_event_seq=last_event_seq+1, state=COALESCE(?2,state), updated_at=?3 WHERE task_id=?4 RETURNING last_event_seq",
        params![token,state,now(),task_id], |r| r.get(0))?;
    tx.execute("INSERT INTO task_events(task_id,event_seq,event_type,payload_json,created_at) VALUES (?1,?2,?3,?4,?5)",
        params![task_id,seq,event_type,serde_json::to_string(&payload)?,now()])?;
    Ok(TaskEvent {
        task_id: task_id.into(),
        event_seq: seq,
        event_type: event_type.into(),
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> (tempfile::TempDir, Project, Chapter) {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::create(dir.path().join("长篇.story"), "试写").unwrap();
        let first = project.chapters().unwrap().remove(0);
        (dir, project, first)
    }

    fn input(chapter: &Chapter, key: &str, prompt: &str) -> TaskInput {
        TaskInput {
            idempotency_key: key.into(),
            chapter_id: chapter.chapter_id.clone(),
            base_revision_id: chapter.current_revision_id.clone(),
            task_type: "beat_write".into(),
            frozen_context: json!({"prompt":prompt,"carryDraft":""}),
        }
    }

    fn save(chapter: &Chapter, base: Option<String>, content: &str, source: &str) -> RevisionInput {
        RevisionInput {
            chapter_id: chapter.chapter_id.clone(),
            base_revision_id: base,
            content: content.into(),
            source: source.into(),
            draft_task_id: None,
        }
    }

    #[test]
    fn schema_two_retains_existing_chapter_and_links_a_shared_scene() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("升级.story");
        let original = Connection::open(&path).unwrap();
        original.execute_batch(SCHEMA).unwrap();
        original.execute("INSERT INTO project_meta(project_id,title,created_at,updated_at) VALUES('old','旧书',?1,?1)", [now()]).unwrap();
        original.execute("INSERT INTO chapters(chapter_id,chapter_order,title,created_at,updated_at) VALUES('original',1,'原章',?1,?1)", [now()]).unwrap();
        drop(original);
        let mut upgraded = Project::open(path).unwrap();
        let second = upgraded.insert_chapter("第二章", None).unwrap();
        let scene = upgraded
            .save_scene(catalog::SceneInput {
                scene_id: None,
                version: None,
                title: "跨章同场".into(),
                details: json!({"location":"渡口"}),
            })
            .unwrap();
        upgraded
            .link_scene("original", Some(&scene.scene_id))
            .unwrap();
        upgraded
            .link_scene(&second.chapter_id, Some(&scene.scene_id))
            .unwrap();
        assert!(upgraded
            .chapters()
            .unwrap()
            .iter()
            .all(|c| c.active_scene_id.as_ref() == Some(&scene.scene_id)));
        assert_eq!(
            upgraded
                .save_scene(catalog::SceneInput {
                    scene_id: Some(scene.scene_id),
                    version: Some(0),
                    title: "并发失效".into(),
                    details: json!({})
                })
                .err()
                .unwrap()
                .status,
            409
        );
    }

    #[test]
    fn story_schema_wal_and_chapter_ids_survive_editing_order() {
        let (_temp, mut project, first) = project();
        let table_count: i64 = project.conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'chapters_fts_%'", [], |r| r.get(0)).unwrap();
        assert_eq!(table_count, 16);
        assert_eq!(
            project
                .conn
                .query_row("PRAGMA journal_mode", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "wal"
        );
        let revision = project
            .save_revision(save(&first, None, "她落了锁。", "manual_edit"))
            .unwrap();
        let last = project.insert_chapter("后续", None).unwrap();
        let middle = project
            .insert_chapter("插入", Some(&first.chapter_id))
            .unwrap();
        assert_eq!(project.chapters().unwrap()[2].chapter_id, last.chapter_id);
        project.move_chapter(&last.chapter_id, -1).unwrap();
        assert_eq!(project.chapters().unwrap()[1].chapter_id, last.chapter_id);
        project.archive_chapter(&first.chapter_id).unwrap();
        assert_eq!(
            project.revisions(&first.chapter_id).unwrap()[0].revision_id,
            revision.revision_id
        );
        assert_eq!(project.chapters().unwrap()[1].chapter_id, middle.chapter_id);
        project.restore_chapter(&first.chapter_id).unwrap();
        assert_eq!(project.chapters().unwrap()[2].chapter_id, first.chapter_id);
    }

    #[test]
    fn native_claim_is_idempotent_and_conflicts_use_409() {
        let (_temp, mut project, first) = project();
        let queued = project
            .create_task(input(&first, "double", "同一提示词"))
            .unwrap();
        let again = project
            .create_task(input(&first, "double", "同一提示词"))
            .unwrap();
        assert_eq!(queued.task_id, again.task_id);
        let conflict = project
            .create_task(input(&first, "double", "不同提示词"))
            .unwrap_err();
        assert_eq!(conflict.status, 409);
        assert!(project.claim_task(&queued.task_id).unwrap().is_some());
        assert!(project.claim_task(&queued.task_id).unwrap().is_none());
    }

    #[test]
    fn disconnected_draft_and_event_sequence_survive_reopen() {
        let (dir, mut project, first) = project();
        let queued = project
            .create_task(input(&first, "disconnect", "从这里写"))
            .unwrap();
        project.claim_task(&queued.task_id).unwrap();
        project.append_token(&queued.task_id, "她抬手").unwrap();
        drop(project);
        let mut reopened = Project::open(dir.path().join("长篇.story")).unwrap();
        let entries = reopened.recover().unwrap();
        assert_eq!(entries[0].partial_output, "她抬手");
        assert_eq!(entries[0].state, "outcome_unknown");
        assert!(reopened.claim_task(&queued.task_id).unwrap().is_none());
        assert!(reopened.recover().unwrap().is_empty());
        let replay = reopened.events(&queued.task_id, 2).unwrap();
        assert_eq!(
            replay.iter().map(|item| item.event_seq).collect::<Vec<_>>(),
            [3, 4]
        );
        assert_eq!(reopened.chapters().unwrap()[0].current_revision_id, None);
    }

    #[test]
    fn buffer_and_ai_draft_never_replace_author_revisions() {
        let (_dir, mut project, first) = project();
        let author = project
            .save_revision(save(&first, None, "  原稿。\n", "manual_edit"))
            .unwrap();
        assert!(
            matches!(project.save_revision(save(&first, None, "过期正文", "manual_edit")), Err(ref e) if e.status == 409)
        );
        let buffer = project
            .save_revision(save(
                &first,
                Some(author.revision_id.clone()),
                "编辑缓冲",
                "auto_buffer",
            ))
            .unwrap();
        assert_eq!(buffer.save_kind, "auto_buffer");
        let chapter = project.chapters().unwrap().remove(0);
        assert_eq!(
            chapter.current_revision_id.as_ref(),
            Some(&author.revision_id)
        );
        let task = project
            .create_task(input(&chapter, "draft", "继续"))
            .unwrap();
        project.claim_task(&task.task_id).unwrap();
        project.append_token(&task.task_id, "补充段落。").unwrap();
        project
            .finish_task(&task.task_id, "completed", None, Some("stop"), None)
            .unwrap();
        let mut accept = save(&chapter, Some(author.revision_id.clone()), "", "ai_gen");
        accept.draft_task_id = Some(task.task_id);
        let appended = project.save_revision(accept).unwrap();
        assert_eq!(appended.content, "  原稿。\n\n补充段落。");
        assert!(project
            .revisions(&chapter.chapter_id)
            .unwrap()
            .iter()
            .any(|entry| entry.content == "  原稿。\n"));
    }
}
