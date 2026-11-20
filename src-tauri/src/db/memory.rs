use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;

use super::{find_task, hash, id, now, Project};
use crate::error::{Result, StudioError};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Audience {
    pub scope: String,
    #[serde(default)]
    pub visible_to: Vec<String>,
    #[serde(default)]
    pub observable_clue: String,
    #[serde(default)]
    pub protected_terms: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Canon {
    #[serde(default)]
    pub entry_id: String,
    #[serde(default)]
    pub version: i64,
    pub category: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub body: String,
    pub audience: Audience,
    pub locked: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
    pub quote: String,
    pub start_offset: usize,
    pub end_offset: usize,
}

impl Anchor {
    pub(super) fn matches(&self, manuscript: &[u16]) -> bool {
        let quote: Vec<u16> = self.quote.encode_utf16().collect();
        !self.quote.trim().is_empty()
            && quote.len() <= 240
            && manuscript.get(self.start_offset..self.end_offset) == Some(quote.as_slice())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub event_type: String,
    pub subjects: Vec<String>,
    pub summary: String,
    pub time: String,
    pub anchor: Anchor,
    pub confidence: String,
    pub objective_routine: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Fact {
    event_id: String,
    source_revision_id: String,
    active_revision_id: Option<String>,
    chapter_id: String,
    chapter_order: i64,
    event_type: String,
    subjects: Vec<String>,
    summary: String,
    time: String,
    anchor: Anchor,
    approval: String,
    confidence: String,
    audience: Audience,
    current: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Knowledge {
    pub character_id: String,
    pub event_id: String,
    pub state: String,
    pub source: String,
    pub since_revision_id: String,
    #[serde(default)]
    pub since_order: i64,
    pub perception: String,
    pub observable_clue: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    #[serde(default)]
    pub note_id: String,
    pub note_type: String,
    pub source_revision_id: String,
    #[serde(default)]
    pub chapter_order: i64,
    pub related_ids: Vec<String>,
    pub body: String,
    pub audience: Audience,
    pub status: String,
    #[serde(default)]
    pub remind_after_chapters: Option<i64>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryRoles {
    pub audit_profile_id: Option<String>,
    pub extraction_profile_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    canon: Vec<Canon>,
    events: Vec<Fact>,
    knowledge: Vec<Knowledge>,
    notes: Vec<Note>,
    roles: MemoryRoles,
    ingested: Vec<(String, String)>,
    pub(super) stamp: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexInput {
    chapter_id: String,
    revision_id: String,
    content: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    chapter_id: String,
    revision_id: String,
    title: String,
    chapter_order: i64,
    excerpt: String,
}

const EVENT_TYPES: &[&str] = &[
    "chapter_summary",
    "location_move",
    "minor_extra",
    "prop_transfer",
    "secret_created",
    "secret_revealed",
    "relation_shift",
    "death",
    "irreversible_change",
    "plot_event",
];
const APPROVED: &str = "('auto_accepted','author_accepted')";

fn invalid(message: &str) -> StudioError {
    StudioError::new(400, message)
}

fn private() -> Audience {
    Audience {
        scope: "narrator_only".into(),
        visible_to: vec![],
        observable_clue: String::new(),
        protected_terms: vec![],
    }
}

fn validate_audience(db: &Connection, audience: &Audience) -> Result<()> {
    if !matches!(
        audience.scope.as_str(),
        "public" | "named_characters" | "narrator_only"
    ) || (audience.scope == "named_characters") == audience.visible_to.is_empty()
        || audience.visible_to.len() > 60
        || audience.protected_terms.len() > 40
        || audience
            .protected_terms
            .iter()
            .any(|term| term.trim().chars().count() < 2 || term.len() > 800)
        || audience.observable_clue.len() > 2400
    {
        return Err(invalid("知情范围或可观察线索有误"));
    }
    for character in &audience.visible_to {
        require_character(db, character)?;
    }
    Ok(())
}

fn require_character(db: &Connection, character_id: &str) -> Result<()> {
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM canon_entries WHERE category='character' AND entry_id=?1)",
        [character_id],
        |r| r.get(0),
    )?;
    if !exists {
        return Err(invalid("请选择设定库中的人物"));
    }
    Ok(())
}

fn read_canon(db: &Connection) -> Result<Vec<Canon>> {
    let mut query = db.prepare(
        "SELECT entry_id,category,name,aliases_json,attributes_json,is_locked,version
        FROM canon_entries ORDER BY category,name,entry_id",
    )?;
    let mut rows = query.query([])?;
    let mut entries = Vec::new();
    while let Some(row) = rows.next()? {
        let details: Value = serde_json::from_str(&row.get::<_, String>(4)?)?;
        entries.push(Canon {
            entry_id: row.get(0)?,
            category: row.get(1)?,
            name: row.get(2)?,
            aliases: serde_json::from_str(&row.get::<_, String>(3)?)?,
            body: details["body"].as_str().unwrap_or_default().into(),
            audience: serde_json::from_value(details["audience"].clone())
                .unwrap_or_else(|_| private()),
            locked: row.get(5)?,
            version: row.get(6)?,
        });
    }
    Ok(entries)
}

fn read_facts(db: &Connection) -> Result<Vec<Fact>> {
    let mut query = db.prepare("SELECT e.event_id,e.source_revision_id,c.chapter_id,c.chapter_order,e.event_type,
        e.subject_ids_json,e.summary,e.world_time_label,COALESCE(e.active_anchor_json,e.evidence_anchor_json),e.approval_status,e.confidence,e.payload_json,
        COALESCE(c.current_revision_id=e.active_revision_id,0) AND c.status!='archived',e.active_revision_id
        FROM story_events e JOIN chapter_revisions r ON r.revision_id=e.source_revision_id
        JOIN chapters c ON c.chapter_id=r.chapter_id ORDER BY c.chapter_order,e.created_at,e.rowid")?;
    let mut rows = query.query([])?;
    let mut facts = Vec::new();
    while let Some(row) = rows.next()? {
        let payload: Value = serde_json::from_str(&row.get::<_, String>(11)?)?;
        facts.push(Fact {
            event_id: row.get(0)?,
            source_revision_id: row.get(1)?,
            active_revision_id: row.get(13)?,
            chapter_id: row.get(2)?,
            chapter_order: row.get(3)?,
            event_type: row.get(4)?,
            subjects: serde_json::from_str(&row.get::<_, String>(5)?)?,
            summary: row.get(6)?,
            time: row.get(7)?,
            anchor: serde_json::from_str(&row.get::<_, String>(8)?)?,
            approval: row.get(9)?,
            confidence: row.get(10)?,
            audience: serde_json::from_value(payload["audience"].clone())
                .unwrap_or_else(|_| private()),
            current: row.get(12)?,
        });
    }
    Ok(facts)
}

impl Project {
    pub fn memory(&self) -> Result<Memory> {
        Self::read_memory(&self.conn)
    }

    pub(super) fn read_memory(conn: &Connection) -> Result<Memory> {
        let mut knowledge_query = conn.prepare("SELECT k.character_id,k.target_event_id,k.epistemic_state,k.knowledge_source,
            k.known_since_revision_id,c.chapter_order,k.perception,k.observable_clues FROM character_knowledge k
            JOIN chapter_revisions r ON r.revision_id=k.known_since_revision_id
            JOIN chapters c ON c.chapter_id=r.chapter_id WHERE c.status!='archived' ORDER BY k.knowledge_id")?;
        let knowledge = knowledge_query
            .query_map([], |r| {
                Ok(Knowledge {
                    character_id: r.get(0)?,
                    event_id: r.get(1)?,
                    state: r.get(2)?,
                    source: r.get(3)?,
                    since_revision_id: r.get(4)?,
                    since_order: r.get(5)?,
                    perception: r.get(6)?,
                    observable_clue: r.get(7)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut note_query = conn.prepare("SELECT n.note_id,n.note_type,n.source_revision_id,c.chapter_order,n.related_ids_json,
            n.content,n.audience_scope,n.visible_to_json,n.status,n.remind_after_chapters FROM soft_notes n
            JOIN chapter_revisions r ON r.revision_id=n.source_revision_id
            JOIN chapters c ON c.chapter_id=r.chapter_id WHERE c.status!='archived' ORDER BY n.created_at,n.rowid")?;
        let mut rows = note_query.query([])?;
        let mut notes = Vec::new();
        while let Some(r) = rows.next()? {
            let visibility: Value = serde_json::from_str(&r.get::<_, String>(7)?)?;
            let mut audience: Audience =
                serde_json::from_value(visibility.clone()).unwrap_or_else(|_| Audience {
                    scope: "narrator_only".into(),
                    visible_to: vec![],
                    observable_clue: String::new(),
                    protected_terms: vec![],
                });
            if visibility.is_object() {
                audience.scope = r.get(6)?;
            }
            notes.push(Note {
                note_id: r.get(0)?,
                note_type: r.get(1)?,
                source_revision_id: r.get(2)?,
                chapter_order: r.get(3)?,
                related_ids: serde_json::from_str(&r.get::<_, String>(4)?)?,
                body: r.get(5)?,
                audience,
                status: r.get(8)?,
                remind_after_chapters: r.get(9)?,
            });
        }
        let roles = conn.query_row(
            "SELECT audit_profile_id,extraction_profile_id FROM project_meta",
            [],
            |r| {
                Ok(MemoryRoles {
                    audit_profile_id: r.get(0)?,
                    extraction_profile_id: r.get(1)?,
                })
            },
        )?;
        let mut ingest = conn.prepare("SELECT task_id,problem FROM memory_ingest")?;
        let ingested = ingest
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let canon = read_canon(conn)?;
        let events = read_facts(conn)?;
        let stamp = if canon.is_empty() && events.is_empty() && notes.is_empty() {
            String::new()
        } else {
            hash(&serde_json::to_string(&(
                &canon, &events, &knowledge, &notes,
            ))?)
        };
        Ok(Memory {
            canon,
            events,
            knowledge,
            notes,
            roles,
            ingested,
            stamp,
        })
    }

    pub fn check_writer_stamp(&self, task: &super::Task) -> Result<()> {
        if matches!(
            task.task_type.as_str(),
            "beat_write" | "ab_write" | "batch_write" | "inspiration"
        ) && task.frozen_context["memoryStamp"]
            .as_str()
            .unwrap_or_default()
            != self.memory()?.stamp
        {
            return Err(StudioError::new(
                409,
                "记忆知情范围已变化，请取消旧任务并重新生成",
            ));
        }
        Ok(())
    }

    pub fn save_canon(&mut self, entry: Canon) -> Result<Canon> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let entry = write_canon(&tx, entry)?;
        tx.commit()?;
        Ok(entry)
    }
}

pub(super) fn write_canon(db: &Connection, mut entry: Canon) -> Result<Canon> {
    entry.name = entry.name.trim().into();
    if !matches!(
        entry.category.as_str(),
        "character" | "prop" | "location" | "world_rule"
    ) || entry.name.is_empty()
        || entry.name.chars().count() > 120
        || entry.body.len() > 40_000
        || entry.aliases.len() > 40
        || entry
            .aliases
            .iter()
            .any(|alias| alias.trim().is_empty() || alias.len() > 360)
    {
        return Err(invalid("请检查设定名称、类别和内容长度"));
    }
    validate_audience(db, &entry.audience)?;
    let attributes = json!({ "body": entry.body, "audience": entry.audience }).to_string();
    let aliases = serde_json::to_string(&entry.aliases)?;
    if entry.entry_id.is_empty() {
        entry.entry_id = id();
        entry.version = 1;
        db.execute("INSERT INTO canon_entries(entry_id,category,name,aliases_json,attributes_json,is_locked,version,updated_at)
                VALUES(?1,?2,?3,?4,?5,?6,1,?7)",
                params![entry.entry_id,entry.category,entry.name,aliases,attributes,entry.locked,now()])?;
    } else {
        let changed = db.execute("UPDATE canon_entries SET name=?1,aliases_json=?2,attributes_json=?3,is_locked=?4,version=version+1,updated_at=?5
                WHERE entry_id=?6 AND version=?7 AND category=?8",
                params![entry.name,aliases,attributes,entry.locked,now(),entry.entry_id,entry.version,entry.category])?;
        if changed == 0 {
            return Err(StudioError::new(409, "设定版本已变化，请重读后编辑"));
        }
        entry.version += 1;
    }
    Ok(entry)
}

impl Project {
    pub fn save_memory_roles(&self, roles: MemoryRoles) -> Result<()> {
        self.conn.execute(
            "UPDATE project_meta SET audit_profile_id=?1,extraction_profile_id=?2",
            params![roles.audit_profile_id, roles.extraction_profile_id],
        )?;
        Ok(())
    }

    pub fn save_note(&self, mut note: Note) -> Result<()> {
        validate_audience(&self.conn, &note.audience)?;
        if !matches!(
            note.note_type.as_str(),
            "foreshadow" | "gag_hook" | "subtext_cue"
        ) || !matches!(
            note.status.as_str(),
            "open" | "paid_off" | "reminded" | "deprecated"
        ) || note.body.trim().is_empty()
            || note.body.len() > 20_000
            || note.related_ids.len() > 60
            || note
                .remind_after_chapters
                .is_some_and(|count| !(1..=1000).contains(&count))
        {
            return Err(invalid("请检查备注类型、状态及正文"));
        }
        let formal: bool = self.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM chapter_revisions WHERE revision_id=?1 AND save_kind='formal_revision')",
            [&note.source_revision_id], |r| r.get(0),
        )?;
        if !formal {
            return Err(invalid("请选择一份正式修订作为来源"));
        }
        if note.note_id.is_empty() {
            note.note_id = id();
        }
        self.conn.execute("INSERT INTO soft_notes(note_id,note_type,related_ids_json,content,audience_scope,visible_to_json,source_revision_id,status,created_at,remind_after_chapters)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10) ON CONFLICT(note_id) DO UPDATE SET note_type=excluded.note_type,
            related_ids_json=excluded.related_ids_json,content=excluded.content,audience_scope=excluded.audience_scope,
            visible_to_json=excluded.visible_to_json,source_revision_id=excluded.source_revision_id,status=excluded.status,remind_after_chapters=excluded.remind_after_chapters",
            params![note.note_id,note.note_type,serde_json::to_string(&note.related_ids)?,note.body,note.audience.scope,
                serde_json::to_string(&note.audience)?,note.source_revision_id,note.status,now(),note.remind_after_chapters])?;
        Ok(())
    }

    pub fn save_knowledge(&self, entry: Knowledge) -> Result<()> {
        require_character(&self.conn, &entry.character_id)?;
        if !matches!(entry.state.as_str(), "knows" | "suspects" | "misbelieves")
            || !matches!(entry.source.as_str(), "observed" | "told" | "inferred")
            || (entry.state != "knows" && entry.perception.trim().is_empty())
            || entry.perception.len() > 3000
            || entry.observable_clue.len() > 2400
        {
            return Err(invalid("请填写人物知情状态和相应认知"));
        }
        let valid: bool = self.conn.query_row(&format!(
            "SELECT EXISTS(SELECT 1 FROM story_events e JOIN chapter_revisions r ON r.revision_id=e.source_revision_id
             JOIN chapters c ON c.chapter_id=r.chapter_id JOIN chapter_revisions s ON s.revision_id=?2
             JOIN chapters d ON d.chapter_id=s.chapter_id WHERE e.event_id=?1 AND e.approval_status IN {APPROVED}
             AND s.save_kind='formal_revision' AND (d.chapter_order>c.chapter_order OR
                 (d.chapter_id=c.chapter_id AND s.rowid>=r.rowid)))"),
            params![entry.event_id,entry.since_revision_id], |r| r.get(0),
        )?;
        if !valid {
            return Err(invalid("知情起点必须对应已经采纳的事件及其后续正式修订"));
        }
        self.conn.execute("INSERT INTO character_knowledge(knowledge_id,character_id,target_event_id,epistemic_state,knowledge_source,
            known_since_revision_id,is_secret,observable_clues,perception,updated_at) VALUES(?1,?2,?3,?4,?5,?6,1,?7,?8,?9)
            ON CONFLICT(character_id,target_event_id) DO UPDATE SET epistemic_state=excluded.epistemic_state,knowledge_source=excluded.knowledge_source,
            known_since_revision_id=excluded.known_since_revision_id,observable_clues=excluded.observable_clues,perception=excluded.perception,
            updated_at=excluded.updated_at",params![id(),entry.character_id,entry.event_id,entry.state,entry.source,
                entry.since_revision_id,entry.observable_clue,entry.perception,now()])?;
        Ok(())
    }

    pub fn ingest_memory(
        &mut self,
        task_id: &str,
        candidates: Vec<Candidate>,
        mut problem: String,
    ) -> Result<()> {
        if candidates.len() > 30 || problem.len() > 800 {
            return Err(invalid("提取结果超过上限"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task = find_task(&tx, task_id)?;
        if task.task_type != "memory_extract" || task.state != "completed" {
            return Err(invalid("只接收已完成的独立记忆提取结果"));
        }
        let inserted = tx.execute(
            "INSERT OR IGNORE INTO memory_ingest(task_id,problem,created_at) VALUES(?1,?2,?3)",
            params![task_id, problem, now()],
        )?;
        if inserted == 0 {
            return Ok(());
        }
        let revision = task
            .base_revision_id
            .ok_or_else(|| invalid("提取任务缺少正式修订"))?;
        let (source,current): (String,bool) = tx.query_row("SELECT r.content,COALESCE(c.current_revision_id=r.revision_id,0) AND c.status!='archived'
            FROM chapter_revisions r JOIN chapters c ON c.chapter_id=r.chapter_id
            WHERE r.revision_id=?1 AND c.chapter_id=?2 AND r.save_kind='formal_revision'",
            params![revision,task.chapter_id], |r| Ok((r.get(0)?,r.get(1)?)))?;
        let manuscript: Vec<u16> = source.encode_utf16().collect();
        let scene_ranges = task.frozen_context["sceneReview"]["ranges"].as_array();
        let known = read_canon(&tx)?;
        let mut candidate_keys = HashSet::new();
        for mut candidate in candidates {
            if !EVENT_TYPES.contains(&candidate.event_type.as_str())
                || !matches!(candidate.confidence.as_str(), "high" | "medium" | "low")
                || candidate.summary.trim().is_empty()
                || candidate.summary.len() > 3000
                || candidate.time.len() > 240
                || candidate.subjects.len() > 15
                || !candidate
                    .subjects
                    .iter()
                    .all(|id| known.iter().any(|entry| &entry.entry_id == id))
                || !candidate.anchor.matches(&manuscript)
                || scene_ranges.is_some_and(|ranges| {
                    !ranges.iter().any(|range| {
                        range["start"]
                            .as_u64()
                            .is_some_and(|start| start <= candidate.anchor.start_offset as u64)
                            && range["end"]
                                .as_u64()
                                .is_some_and(|end| end >= candidate.anchor.end_offset as u64)
                    })
                })
            {
                problem = "部分候选缺少可核验的精确原文锚点或必填字段".into();
                continue;
            }
            candidate.subjects.sort_unstable();
            candidate.subjects.dedup();
            if !candidate_keys.insert((
                candidate.event_type.clone(),
                candidate.anchor.start_offset,
                candidate.summary.clone(),
            )) {
                continue;
            }
            let anchor = serde_json::to_string(&candidate.anchor)?;
            let identical: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM story_events WHERE active_revision_id=?1
                AND event_type=?2 AND COALESCE(active_anchor_json,evidence_anchor_json)=?3 AND summary=?4)",
                params![revision, candidate.event_type, anchor, candidate.summary],
                |r| r.get(0),
            )?;
            if identical {
                continue;
            }
            let collision = known.iter().any(|entry| {
                entry.locked
                    && (candidate.subjects.contains(&entry.entry_id)
                        || std::iter::once(&entry.name)
                            .chain(&entry.aliases)
                            .any(|name| candidate.summary.contains(name)))
            });
            let subjects = serde_json::to_string(&candidate.subjects)?;
            let temporal_conflict: bool = tx.query_row(
                &format!("SELECT EXISTS(SELECT 1 FROM story_events e JOIN chapter_revisions r ON r.revision_id=e.source_revision_id
                    JOIN chapters c ON c.current_revision_id=e.active_revision_id
                  WHERE e.event_type=?1 AND e.subject_ids_json=?2 AND e.world_time_label=?3
                  AND e.summary<>?4 AND e.approval_status IN {APPROVED})"),
                params![candidate.event_type,subjects,candidate.time,candidate.summary], |r| r.get(0))?;
            let irreversible = [
                "已死",
                "死亡",
                "身亡",
                "去世",
                "绝交",
                "分手",
                "背叛",
                "调包",
                "秘密",
                "不可逆",
                "身份揭露",
            ]
            .iter()
            .any(|term| candidate.summary.contains(term) || candidate.anchor.quote.contains(term));
            let routine = current
                && candidate.objective_routine
                && candidate.confidence == "high"
                && !collision
                && !irreversible
                && (candidate.event_type == "chapter_summary" || !temporal_conflict)
                && matches!(
                    candidate.event_type.as_str(),
                    "chapter_summary" | "location_move" | "minor_extra" | "prop_transfer"
                );
            // Only verbatim routine actions wholly inside a witnessed scene can inherit its audience.
            let mut audience = private();
            if routine
                && candidate.summary.trim() == candidate.anchor.quote.trim()
                && matches!(
                    candidate.event_type.as_str(),
                    "location_move" | "prop_transfer"
                )
            {
                let witness = task.frozen_context["viewpoints"]
                    .as_array()
                    .and_then(|ranges| {
                        ranges
                            .iter()
                            .find(|range| {
                                range["start"].as_u64().is_some_and(|start| {
                                    start <= candidate.anchor.start_offset as u64
                                }) && range["end"]
                                    .as_u64()
                                    .is_some_and(|end| end >= candidate.anchor.end_offset as u64)
                            })
                            .and_then(|range| range["povCharacterId"].as_str())
                    })
                    .and_then(|id| {
                        known
                            .iter()
                            .find(|entry| entry.category == "character" && entry.entry_id == id)
                    });
                if let Some(person) = witness.filter(|person| {
                    std::iter::once(&person.name)
                        .chain(&person.aliases)
                        .any(|name| candidate.anchor.quote.contains(name))
                }) {
                    audience.scope = "named_characters".into();
                    audience.visible_to.push(person.entry_id.clone());
                }
            }
            tx.execute("INSERT INTO story_events(event_id,source_revision_id,event_type,subject_ids_json,summary,payload_json,
                evidence_anchor_json,confidence,approval_status,world_time_label,created_at,active_revision_id) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?2)",
                params![id(),revision,candidate.event_type,subjects,candidate.summary,
                    json!({ "unclassifiedRoutine": routine && audience.scope == "narrator_only",
                        "audience": audience, "taskId": task_id }).to_string(),
                    anchor,candidate.confidence,if routine { "auto_accepted" } else { "pending" },candidate.time,now()])?;
        }
        tx.execute(
            "UPDATE memory_ingest SET problem=?1 WHERE task_id=?2",
            params![problem, task_id],
        )?;
        update_sync(&tx, &revision, &task.chapter_id)?;
        tx.commit()?;
        Ok(())
    }

    pub fn review_fact(
        &mut self,
        event_id: &str,
        action: &str,
        audience: Option<Audience>,
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (revision,chapter,approval,payload,current): (String,String,String,String,bool) = tx.query_row(
            "SELECT COALESCE(e.active_revision_id,r.revision_id),c.chapter_id,e.approval_status,e.payload_json,
            COALESCE(c.current_revision_id=e.active_revision_id,0) AND c.status!='archived'
            FROM story_events e JOIN chapter_revisions r ON r.revision_id=e.source_revision_id
            JOIN chapters c ON c.chapter_id=r.chapter_id WHERE e.event_id=?1", [event_id],
            |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?)))?;
        let mut payload: Value = serde_json::from_str(&payload)?;
        let state = match action {
            "accept"
                if current
                    && matches!(
                        approval.as_str(),
                        "pending" | "auto_accepted" | "author_accepted"
                    ) =>
            {
                let audience = audience.ok_or_else(|| invalid("请选择事件知情范围"))?;
                validate_audience(&tx, &audience)?;
                payload["audience"] = serde_json::to_value(audience)?;
                payload["unclassifiedRoutine"] = Value::Bool(false);
                "author_accepted"
            }
            "reject" if approval == "pending" => "rejected",
            "undo" if matches!(approval.as_str(), "auto_accepted" | "author_accepted") => {
                "rejected"
            }
            _ => {
                return Err(StudioError::new(
                    409,
                    "事件状态已变化或原文已被替换，请重新提取",
                ))
            }
        };
        tx.execute(
            "UPDATE story_events SET approval_status=?1,payload_json=?2 WHERE event_id=?3",
            params![state, payload.to_string(), event_id],
        )?;
        update_sync(&tx, &revision, &chapter)?;
        tx.commit()?;
        Ok(())
    }

    pub fn memory_refresh(&self, chapter_id: &str, revision_id: &str) -> Result<()> {
        update_sync(&self.conn, revision_id, chapter_id)
    }

    pub fn stale_indexes(&self) -> Result<Vec<IndexInput>> {
        let mut statement = self.conn.prepare("SELECT c.chapter_id,r.revision_id,r.content FROM chapters c
            JOIN chapter_revisions r ON r.revision_id=c.current_revision_id WHERE c.indexed_revision_id IS NOT c.current_revision_id
            ORDER BY c.chapter_order LIMIT 12")?;
        let rows = statement
            .query_map([], |r| {
                Ok(IndexInput {
                    chapter_id: r.get(0)?,
                    revision_id: r.get(1)?,
                    content: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn index_chapter(
        &mut self,
        chapter_id: &str,
        revision_id: &str,
        bigrams: &str,
    ) -> Result<bool> {
        if bigrams.len() > 42_000_000 {
            return Err(invalid("章节检索索引超出上限"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let rowid: Option<i64> = tx
            .query_row(
                "SELECT rowid FROM chapters WHERE chapter_id=?1 AND current_revision_id=?2
            AND indexed_revision_id IS NOT current_revision_id",
                params![chapter_id, revision_id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(rowid) = rowid else {
            return Ok(false);
        };
        tx.execute("DELETE FROM chapters_fts WHERE rowid=?1", [rowid])?;
        if !bigrams.is_empty() {
            tx.execute(
                "INSERT INTO chapters_fts(rowid,chapter_id,bigram_text) VALUES(?1,?2,?3)",
                params![rowid, chapter_id, bigrams],
            )?;
        }
        tx.execute(
            "UPDATE chapters SET indexed_revision_id=?1 WHERE rowid=?2",
            params![revision_id, rowid],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn search_text(
        &self,
        phrase: &str,
        exact: &str,
        after_order: Option<i64>,
    ) -> Result<Vec<SearchHit>> {
        if phrase.len() > 1800
            || phrase.is_empty()
            || exact.is_empty()
            || exact.len() > 480
            || phrase
                .bytes()
                .any(|b| b != b' ' && b != b'h' && !b.is_ascii_hexdigit())
        {
            return Err(invalid("正文搜索需要连续的两个或以上汉字"));
        }
        let query = format!("\"{phrase}\"");
        let mut statement = self.conn.prepare_cached("SELECT c.chapter_id,r.revision_id,c.title,c.chapter_order,r.content
            FROM chapters_fts JOIN chapters c ON c.rowid=chapters_fts.rowid
            JOIN chapter_revisions r ON r.revision_id=c.current_revision_id
            WHERE chapters_fts MATCH ?1 AND c.indexed_revision_id=c.current_revision_id AND c.status!='archived'
            AND c.chapter_order>COALESCE(?2,0)
            ORDER BY c.chapter_order LIMIT 50")?;
        let mut rows = statement.query(params![query, after_order])?;
        let mut hits = Vec::new();
        while let Some(r) = rows.next()? {
            let content = r
                .get_ref(4)?
                .as_str()
                .map_err(|_| invalid("章节正文格式有误"))?;
            let Some(at) = content.find(exact) else {
                continue;
            };
            let start = content[..at]
                .char_indices()
                .rev()
                .nth(24)
                .map_or(0, |(pos, _)| pos);
            let end = content[at + exact.len()..]
                .char_indices()
                .nth(38)
                .map_or(content.len(), |(pos, _)| at + exact.len() + pos);
            hits.push(SearchHit {
                chapter_id: r.get(0)?,
                revision_id: r.get(1)?,
                title: r.get(2)?,
                chapter_order: r.get(3)?,
                excerpt: content[start..end].to_owned(),
            });
        }
        Ok(hits)
    }
}

fn update_sync(db: &Connection, revision: &str, chapter: &str) -> Result<()> {
    let state: String = db.query_row("SELECT CASE
        WHEN EXISTS(SELECT 1 FROM generation_tasks WHERE base_revision_id=?1 AND task_type IN('memory_extract','consistency_check')
            AND json_extract(frozen_context_json,'$.stagedDraftId') IS NULL AND state IN('running','streaming')) THEN 'processing'
        WHEN EXISTS(SELECT 1 FROM story_events WHERE active_revision_id=?1 AND approval_status='pending')
            OR COALESCE((SELECT m.problem FROM memory_ingest m JOIN generation_tasks g ON g.task_id=m.task_id
                WHERE g.base_revision_id=?1 ORDER BY g.rowid DESC LIMIT 1),'')<>'' THEN 'needs_confirm'
        WHEN EXISTS(SELECT 1 FROM memory_ingest m JOIN generation_tasks g ON g.task_id=m.task_id
                WHERE g.base_revision_id=?1 AND m.problem='' AND json_extract(g.frozen_context_json,'$.sceneReview') IS NULL) THEN 'synced'
        ELSE 'pending' END", [revision], |r| r.get(0))?;
    db.execute(
        "UPDATE chapters SET memory_sync_status=?1,memory_version=memory_version+1
        WHERE chapter_id=?2 AND current_revision_id=?3",
        params![state, chapter, revision],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Revision, RevisionInput, TaskInput};
    use std::time::Instant;

    fn manuscript() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let project = Project::create(dir.path().join("验收.story"), "渡口").unwrap();
        (dir, project)
    }

    fn save(
        project: &mut Project,
        chapter_id: &str,
        base: Option<String>,
        content: String,
    ) -> Revision {
        project
            .save_revision(RevisionInput {
                chapter_id: chapter_id.into(),
                base_revision_id: base,
                source: "manual_edit".into(),
                content,
                draft_task_id: None,
                edit_level: None,
                canon_confirmed: false,
            })
            .unwrap()
    }

    fn grams(text: &str) -> String {
        text.chars()
            .collect::<Vec<_>>()
            .windows(2)
            .map(|pair| format!("h{:06x}{:06x}", pair[0] as u32, pair[1] as u32))
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn contentless_bigram_search_meets_gate_on_one_million_han_characters() {
        let (_dir, mut project) = manuscript();
        let filler = "石桥下的船晚些才开寻常人家都把木门掩好等雨过".repeat(500);
        let mut order = project.chapters().unwrap().remove(0);
        let size = filler.chars().count() * 100;
        assert!(size >= 1_000_000);
        for index in 0..100 {
            if index > 0 {
                order = project
                    .insert_chapter("雨夜", Some(&order.chapter_id))
                    .unwrap();
            }
            let extra = if index == 55 { "周宁" } else { "周、宁" };
            let revision = save(
                &mut project,
                &order.chapter_id,
                None,
                format!("{filler}。{extra}"),
            );
            let index_text = [
                grams(&filler),
                if index == 55 {
                    grams(extra)
                } else {
                    String::new()
                },
            ]
            .join(" gap ");
            assert!(project
                .index_chapter(&order.chapter_id, &revision.revision_id, &index_text)
                .unwrap());
        }
        let keyword = grams("周宁");
        let before = Instant::now();
        let hits = project.search_text(&keyword, "周宁", None).unwrap();
        let cold_ms = before.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].chapter_order, 56);
        let mut timings: Vec<_> = (0..80)
            .map(|_| {
                let start = Instant::now();
                let hits = project.search_text(&keyword, "周宁", None).unwrap();
                assert_eq!(hits.len(), 1);
                start.elapsed().as_secs_f64() * 1000.0
            })
            .collect();
        timings.sort_by(f64::total_cmp);
        let median = timings[40];
        let p95 = timings[75];
        eprintln!("Han count={size}, cold={cold_ms:.3}ms, median={median:.3}ms, p95={p95:.3}ms");
        assert!(p95 < 5.0);
        assert_eq!(
            project
                .search_text(&grams("宁石"), "宁石", None)
                .unwrap()
                .len(),
            0
        );
        assert_eq!(
            project
                .conn
                .query_row("PRAGMA quick_check", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ok"
        );
    }

    #[test]
    fn evidence_policies_are_atomic_and_invalid_offsets_never_enter_memory() {
        let (_dir, mut project) = manuscript();
        let chapter = project.chapters().unwrap().remove(0);
        let revision = save(
            &mut project,
            &chapter.chapter_id,
            None,
            "🗝雨停了。他走到前楼。进门才知周宁已死。".into(),
        );
        let candidate = |event_type: &str, quote: &str, shift: usize| {
            let byte = revision.content.find(quote).unwrap();
            let offset = revision.content[..byte].encode_utf16().count() + shift;
            Candidate {
                event_type: event_type.into(),
                subjects: vec![],
                summary: quote.into(),
                time: "".into(),
                confidence: "high".into(),
                objective_routine: true,
                anchor: Anchor {
                    quote: quote.into(),
                    start_offset: offset,
                    end_offset: offset + quote.encode_utf16().count(),
                },
            }
        };
        let input = |kind: &str| TaskInput {
            idempotency_key: format!("case-{kind}"),
            chapter_id: chapter.chapter_id.clone(),
            base_revision_id: Some(revision.revision_id.clone()),
            task_type: kind.into(),
            frozen_context: json!({"prompt":"独立调用","requestBody":{}}),
        };
        let extract = project.create_task(input("memory_extract")).unwrap();
        let audit = project.create_task(input("consistency_check")).unwrap();
        for task in [&extract, &audit] {
            project.claim_task(&task.task_id).unwrap();
            project
                .finish_task(&task.task_id, "completed", None, Some("stop"), None)
                .unwrap();
        }
        assert_eq!(
            project
                .ingest_memory(&audit.task_id, vec![], String::new())
                .unwrap_err()
                .status,
            400
        );
        project
            .ingest_memory(
                &extract.task_id,
                vec![
                    candidate("location_move", "他走到前楼", 0),
                    candidate("death", "周宁已死", 0),
                    candidate("chapter_summary", "雨停了", 1),
                ],
                String::new(),
            )
            .unwrap();
        project
            .ingest_memory(&extract.task_id, vec![], String::new())
            .unwrap();
        let page = project.memory().unwrap();
        assert_eq!(page.events.len(), 2);
        assert_eq!(page.ingested.len(), 1);
        assert!(!page.ingested[0].1.is_empty());
        let objective = page
            .events
            .iter()
            .find(|event| event.event_type == "location_move")
            .unwrap();
        let death = page
            .events
            .iter()
            .find(|event| event.event_type == "death")
            .unwrap();
        assert_eq!(objective.approval, "auto_accepted");
        assert_eq!(death.approval, "pending");
        assert!(page.events.iter().all(|event| event
            .anchor
            .matches(&revision.content.encode_utf16().collect::<Vec<_>>())));
        project
            .review_fact(&objective.event_id, "undo", None)
            .unwrap();
        project
            .review_fact(&death.event_id, "accept", Some(private()))
            .unwrap();
        let reopened = Project::open(std::path::PathBuf::from(&project.info.path))
            .unwrap()
            .memory()
            .unwrap();
        assert!(reopened
            .events
            .iter()
            .any(|event| event.approval == "rejected"));
        assert!(reopened
            .events
            .iter()
            .any(|event| event.event_type == "death" && event.approval == "author_accepted"));
    }
}
