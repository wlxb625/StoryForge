use super::{current_revision, find_task, id, now, Project, Revision, RevisionInput};
use crate::error::{Result, StudioError};
use rusqlite::{params, Connection, OptionalExtension, Row, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::memory::Anchor;

pub fn resolve_level<'a>(
    db: &Connection,
    base: Option<&str>,
    next: &str,
    declared: Option<&'a str>,
    confirmed: bool,
) -> Result<&'a str> {
    let level = if declared == Some("canon") {
        "canon"
    } else if let Some(base) = base {
        let original: String = db.query_row(
            "SELECT content FROM chapter_revisions WHERE revision_id=?1",
            [base],
            |r| r.get(0),
        )?;
        let mut anchors = db.prepare("SELECT COALESCE(active_anchor_json,evidence_anchor_json)
            FROM story_events WHERE active_revision_id=?1 AND approval_status='author_accepted'
            AND event_type IN('death','secret_created','secret_revealed','relation_shift','irreversible_change')")?;
        let high_risk = anchors
            .query_map([base], |r| r.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .map(|raw| serde_json::from_str::<Anchor>(&raw))
            .collect::<serde_json::Result<Vec<_>>>()?
            .iter()
            .any(|anchor| original.contains(&anchor.quote) && !next.contains(&anchor.quote));
        if high_risk && surface_form(&original) != surface_form(next) {
            "canon"
        } else if declared == Some("surface") || surface_form(&original) == surface_form(next) {
            "surface"
        } else {
            "semantic"
        }
    } else {
        "semantic"
    };
    if level == "canon" && !confirmed {
        return Err(StudioError::new(
            409,
            "此修订会改变重要已确认事实，请在保存修订时选择设定变更并确认",
        ));
    }
    Ok(level)
}

fn surface_form(content: &str) -> String {
    content
        .chars()
        .filter(|ch| {
            !ch.is_whitespace()
                && !ch.is_ascii_punctuation()
                && !matches!(
                    *ch,
                    '，' | '。'
                        | '、'
                        | '；'
                        | '：'
                        | '？'
                        | '！'
                        | '“'
                        | '”'
                        | '‘'
                        | '’'
                        | '（'
                        | '）'
                        | '《'
                        | '》'
                        | '…'
                        | '—'
                )
        })
        .collect()
}

pub fn relink_evidence(
    db: &Connection,
    base: &str,
    revision: &Revision,
    appended: bool,
) -> Result<()> {
    let mut statement = db.prepare(
        "SELECT event_id,COALESCE(active_anchor_json,evidence_anchor_json)
        FROM story_events WHERE active_revision_id=?1 AND approval_status!='rejected'
        AND (event_type!='chapter_summary' OR ?2)",
    )?;
    let pairs = statement
        .query_map(params![base, revision.edit_level == "surface"], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let utf16: Vec<u16> = revision.content.encode_utf16().collect();
    for (event_id, original) in pairs {
        let mut anchor: Anchor = serde_json::from_str(&original)?;
        let mut same_position = anchor.matches(&utf16);
        if !same_position && !appended {
            let mut positions = revision.content.match_indices(&anchor.quote);
            if let Some((byte, _)) = positions.next() {
                if positions.next().is_none() {
                    anchor.start_offset = revision.content[..byte].encode_utf16().count();
                    anchor.end_offset = anchor.start_offset + anchor.quote.encode_utf16().count();
                    same_position = true;
                }
            }
        }
        if same_position {
            db.execute("UPDATE story_events SET active_revision_id=?1,active_anchor_json=?2 WHERE event_id=?3",
                params![revision.revision_id,serde_json::to_string(&anchor)?,event_id])?;
        }
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub expected: String,
    pub content_hash: String,
}

#[derive(Deserialize)]
struct Replacement {
    replacement: String,
}

pub fn replace_selection(original: &str, span: &Span, replacement: &str) -> Result<String> {
    let utf: Vec<u16> = original.encode_utf16().collect();
    let expected: Vec<u16> = span.expected.encode_utf16().collect();
    if expected.is_empty()
        || expected.len() > 1800
        || utf.get(span.start..span.end) != Some(expected.as_slice())
        || span.content_hash != super::hash(original)
    {
        return Err(StudioError::new(
            409,
            "所选文字或正文版本已变化，请重新选取范围",
        ));
    }
    let head = String::from_utf16(&utf[..span.start])
        .map_err(|_| StudioError::new(409, "选区边界已变化"))?;
    let tail = String::from_utf16(&utf[span.end..])
        .map_err(|_| StudioError::new(409, "选区边界已变化"))?;
    Ok(head + replacement + &tail)
}

impl Project {
    pub fn apply_polish(&mut self, task_id: &str, confirm_canon: bool) -> Result<Revision> {
        let task = find_task(&self.conn, task_id)?;
        if task.task_type != "polish_span"
            || task.state != "completed"
            || current_revision(&self.conn, &task.chapter_id)? != task.base_revision_id
        {
            return Err(StudioError::new(409, "建议过期或生成尚未完成，请重选原文"));
        }
        let span: Span = serde_json::from_value(task.frozen_context["span"].clone())?;
        let base = task
            .base_revision_id
            .ok_or_else(|| StudioError::new(409, "缺少正式修订"))?;
        let manuscript: String = self.conn.query_row(
            "SELECT content FROM chapter_revisions WHERE revision_id=?1",
            [&base],
            |r| r.get(0),
        )?;
        let raw = task.partial_output.trim();
        let payload = raw
            .strip_prefix("```json")
            .or_else(|| raw.strip_prefix("```"))
            .and_then(|stripped| stripped.strip_suffix("```"))
            .unwrap_or(raw)
            .trim();
        let suggestion: Replacement = serde_json::from_str(payload)?;
        if suggestion.replacement.encode_utf16().count() > 3600 {
            return Err(StudioError::new(400, "建议超过选区编辑长度"));
        }
        let updated = replace_selection(&manuscript, &span, &suggestion.replacement)?;
        self.save_revision(RevisionInput {
            chapter_id: task.chapter_id,
            base_revision_id: Some(base),
            content: updated,
            source: "polish_diff".into(),
            draft_task_id: None,
            edit_level: None,
            canon_confirmed: confirm_canon,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Milestone {
    id: String,
    text: String,
    done: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Arc {
    #[serde(default)]
    pub arc_id: String,
    pub title: String,
    pub rough_goal: String,
    pub expected_chapters: i64,
    pub milestones: Vec<Milestone>,
    pub pivot_instruction: String,
    pub pivot_remaining: i64,
    pub pivot_start_order: i64,
    pub status: String,
    pub version: i64,
    pub writer_visible: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImpactNote {
    note_id: String,
    body: String,
    note_type: String,
    chapter_id: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PivotPreview {
    version: i64,
    previous_goal: String,
    suggested: i64,
    notes: Vec<ImpactNote>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pivot {
    arc_id: String,
    version: i64,
    next_goal: String,
    instruction: String,
    transition_chapters: i64,
    note_ids: Vec<String>,
}

const ARC_COLUMNS: &str = "arc_id,title,rough_goal_text,expected_chapters,milestones_json,pivot_instruction,pivot_remaining_chapters,pivot_start_order,status,version,writer_visible";

fn map_arc(r: &Row<'_>) -> rusqlite::Result<(Arc, String)> {
    Ok((
        Arc {
            arc_id: r.get(0)?,
            title: r.get(1)?,
            rough_goal: r.get(2)?,
            expected_chapters: r.get(3)?,
            milestones: Vec::new(),
            pivot_instruction: r.get(5)?,
            pivot_remaining: r.get(6)?,
            pivot_start_order: r.get(7)?,
            status: r.get(8)?,
            version: r.get(9)?,
            writer_visible: r.get(10)?,
        },
        r.get(4)?,
    ))
}

fn read_arc(db: &Connection, id: &str) -> Result<Arc> {
    let (mut arc, milestones) = db.query_row(
        &format!("SELECT {ARC_COLUMNS} FROM stage_arcs WHERE arc_id=?1"),
        [id],
        map_arc,
    )?;
    arc.milestones = serde_json::from_str(&milestones)?;
    Ok(arc)
}

fn impact_notes(db: &Connection, arc_id: &str) -> Result<Vec<ImpactNote>> {
    let mut statement = db.prepare("SELECT n.note_id,n.content,n.note_type,c.chapter_id FROM soft_notes n
        JOIN chapter_revisions r ON r.revision_id=n.source_revision_id JOIN chapters c ON c.chapter_id=r.chapter_id
        WHERE n.status IN('open','reminded') AND c.status!='archived' AND (c.arc_id=?1 OR c.arc_id IS NULL)
        ORDER BY c.chapter_order,n.note_id")?;
    let notes = statement
        .query_map([arc_id], |r| {
            Ok(ImpactNote {
                note_id: r.get(0)?,
                body: r.get(1)?,
                note_type: r.get(2)?,
                chapter_id: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(notes)
}

impl Project {
    pub fn arcs(&self) -> Result<Vec<Arc>> {
        let mut query = self.conn.prepare(&format!(
            "SELECT {ARC_COLUMNS} FROM stage_arcs ORDER BY arc_order"
        ))?;
        let rows = query
            .query_map([], map_arc)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        rows.into_iter()
            .map(|(mut arc, milestones)| {
                arc.milestones = serde_json::from_str(&milestones)?;
                Ok(arc)
            })
            .collect()
    }

    pub fn write_arc(&mut self, mut arc: Arc) -> Result<Arc> {
        if arc.title.trim().is_empty()
            || arc.title.chars().count() > 120
            || arc.rough_goal.trim().is_empty()
            || arc.rough_goal.len() > 12_000
            || !(1..=999).contains(&arc.expected_chapters)
            || arc.milestones.len() > 40
            || arc.milestones.iter().any(|step| {
                step.id.len() > 60 || step.text.trim().is_empty() || step.text.len() > 1200
            })
            || !matches!(arc.status.as_str(), "active" | "completed")
        {
            return Err(StudioError::new(
                400,
                "请检查阶段名、目标、里程碑和预计章数",
            ));
        }
        let steps = serde_json::to_string(&arc.milestones)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if arc.arc_id.is_empty() {
            arc.arc_id = id();
            tx.execute("INSERT INTO stage_arcs(arc_id,arc_order,title,rough_goal_text,expected_chapters,milestones_json,status,writer_visible,updated_at)
                VALUES(?1,(SELECT COALESCE(MAX(arc_order),0)+1 FROM stage_arcs),?2,?3,?4,?5,?6,?7,?8)",
                params![arc.arc_id,arc.title,arc.rough_goal,arc.expected_chapters,steps,arc.status,arc.writer_visible,now()])?;
        } else {
            let changed = tx.execute("UPDATE stage_arcs SET title=?1,expected_chapters=?2,milestones_json=?3,status=?4,
                writer_visible=?5,updated_at=?6,version=version+1 WHERE arc_id=?7 AND version=?8 AND rough_goal_text=?9",
                params![arc.title,arc.expected_chapters,steps,arc.status,arc.writer_visible,now(),arc.arc_id,arc.version,arc.rough_goal])?;
            if changed == 0 {
                return Err(StudioError::new(
                    409,
                    "阶段已有更新，目标变更需通过改纲预览",
                ));
            }
        }
        let saved = read_arc(&tx, &arc.arc_id)?;
        tx.commit()?;
        Ok(saved)
    }

    pub fn assign_arc(&self, chapter: &str, arc: Option<&str>) -> Result<()> {
        let changed = self.conn.execute("UPDATE chapters SET arc_id=?1,updated_at=?2 WHERE chapter_id=?3 AND status!='archived'",
            params![arc,now(),chapter])?;
        if changed == 0 {
            return Err(StudioError::new(404, "找不到此章节"));
        }
        Ok(())
    }

    pub fn preview_pivot(&self, arc_id: &str) -> Result<PivotPreview> {
        let arc = read_arc(&self.conn, arc_id)?;
        let notes = impact_notes(&self.conn, arc_id)?;
        let open = arc.milestones.iter().filter(|step| !step.done).count();
        let suggested = ((notes.len() + open + 1) / 2) as i64;
        let suggested = suggested.min(arc.expected_chapters).min(12);
        Ok(PivotPreview {
            version: arc.version,
            previous_goal: arc.rough_goal,
            suggested,
            notes,
        })
    }

    pub fn apply_pivot(&mut self, pivot: Pivot) -> Result<Arc> {
        if pivot.next_goal.trim().is_empty()
            || pivot.next_goal.len() > 12_000
            || pivot.instruction.len() > 12_000
            || !(0..=999).contains(&pivot.transition_chapters)
        {
            return Err(StudioError::new(400, "请填写新目标及有效的过渡章数"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let arc = read_arc(&tx, &pivot.arc_id)?;
        let notes: Vec<_> = impact_notes(&tx, &pivot.arc_id)?
            .into_iter()
            .map(|note| note.note_id)
            .collect();
        if arc.version != pivot.version || notes != pivot.note_ids {
            return Err(StudioError::new(
                409,
                "目标或相关伏笔已有更新，请重新预览改纲",
            ));
        }
        let latest: Option<i64> = tx.query_row(
            "SELECT MAX(chapter_order) FROM chapters
            WHERE arc_id=?1 AND current_revision_id IS NOT NULL AND status!='archived'",
            [&pivot.arc_id],
            |r| r.get(0),
        )?;
        let first: Option<i64> = tx
            .query_row(
                "SELECT MIN(chapter_order) FROM chapters WHERE arc_id=?1 AND status!='archived'",
                [&pivot.arc_id],
                |r| r.get(0),
            )
            .optional()?
            .flatten();
        let start = latest.map(|value| value + 1).or(first).unwrap_or(1);
        tx.execute("INSERT INTO arc_pivots(pivot_id,arc_id,previous_goal,next_goal,instruction,future_chapters,affected_notes_json,created_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![id(),pivot.arc_id,arc.rough_goal,pivot.next_goal,pivot.instruction,
                pivot.transition_chapters,serde_json::to_string(&notes)?,now()])?;
        tx.execute("UPDATE stage_arcs SET rough_goal_text=?1,pivot_instruction=?2,pivot_remaining_chapters=?3,
            pivot_start_order=?4,version=version+1,updated_at=?5 WHERE arc_id=?6",
            params![pivot.next_goal,pivot.instruction,pivot.transition_chapters,start,now(),pivot.arc_id])?;
        let saved = read_arc(&tx, &pivot.arc_id)?;
        tx.commit()?;
        Ok(saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stale_offsets_hash_and_revision_are_rejected() {
        let source = "她看见🗝，慢慢把门推开。";
        let span = Span {
            start: 3,
            end: 5,
            expected: "🗝".into(),
            content_hash: super::super::hash(source),
        };
        assert_eq!(
            replace_selection(source, &span, "铜匙").unwrap(),
            "她看见铜匙，慢慢把门推开。"
        );
        assert_eq!(
            replace_selection("她看见🗝，立刻把门推开。", &span, "铜匙")
                .unwrap_err()
                .status,
            409
        );
        let wrong = Span { start: 4, ..span };
        assert_eq!(
            replace_selection(source, &wrong, "铜匙")
                .unwrap_err()
                .status,
            409
        );
    }

    #[test]
    fn phase_four_fixture_preserves_future_prose_and_surface_revision_memory() {
        let dir = tempfile::tempdir().unwrap();
        let mut book = Project::create(dir.path().join("改纲.story"), "改纲验收").unwrap();
        let first = book.chapters().unwrap().remove(0);
        let mut previous = first;
        let mut chapter5 = String::new();
        let mut chapter6 = String::new();
        for chapter in 2..=6 {
            previous = book
                .insert_chapter(&format!("第{chapter}章"), Some(&previous.chapter_id))
                .unwrap();
            if chapter == 5 {
                chapter5 = previous.chapter_id.clone();
            }
            if chapter == 6 {
                chapter6 = previous.chapter_id.clone();
            }
        }
        let write =
            |chapter: &str, base: Option<String>, text: &str, level: Option<&str>| RevisionInput {
                chapter_id: chapter.into(),
                base_revision_id: base,
                content: text.into(),
                source: "manual_edit".into(),
                draft_task_id: None,
                edit_level: level.map(str::to_owned),
                canon_confirmed: false,
            };
        let old = book
            .save_revision(write(
                &chapter5,
                None,
                "晚些时侯。母亲把铜匙留在北门。",
                None,
            ))
            .unwrap();
        let future = book
            .save_revision(write(&chapter6, None, "他在北门摸到了那把铜匙。", None))
            .unwrap();
        book.index_chapter(&chapter6, &future.revision_id, "h0094dc00532a")
            .unwrap();
        book.conn.execute("UPDATE chapters SET memory_sync_status='synced',memory_version=4 WHERE chapter_id=?1",[&chapter5]).unwrap();
        let surface = book
            .save_revision(write(
                &chapter5,
                Some(old.revision_id),
                "晚些时候。母亲把铜匙留在北门。",
                Some("surface"),
            ))
            .unwrap();
        let state: (String, i64) = book
            .conn
            .query_row(
                "SELECT memory_sync_status,memory_version FROM chapters WHERE chapter_id=?1",
                [&chapter5],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, ("synced".into(), 4));
        assert_eq!(surface.edit_level, "surface");
        let denied = book.save_revision(write(
            &chapter5,
            Some(surface.revision_id.clone()),
            "篡改设定",
            Some("canon"),
        ));
        assert_eq!(denied.err().unwrap().status, 409);
        let edited = book
            .save_revision(write(
                &chapter5,
                Some(surface.revision_id),
                "晚些时候。母亲把铜匙转交给周宁。",
                None,
            ))
            .unwrap();
        assert_eq!(edited.edit_level, "semantic");
        let hits = book.search_text("h0094dc00532a", "铜匙", Some(5)).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(
            serde_json::to_value(&hits[0]).unwrap()["chapterId"],
            chapter6
        );
        let untouched = book.revisions(&chapter6).unwrap();
        assert_eq!(untouched.len(), 1);
        assert_eq!(untouched[0].content, future.content);
        assert_eq!(untouched[0].revision_id, future.revision_id);

        let task = book.create_task(super::super::TaskInput { idempotency_key:"polish".into(),chapter_id:chapter5.clone(),
            base_revision_id:Some(edited.revision_id.clone()),task_type:"polish_span".into(),
            frozen_context:json!({"prompt":"改词","span":{"start":0,"end":4,"expected":"晚些时候","contentHash":edited.content_hash}}) }).unwrap();
        book.claim_task(&task.task_id).unwrap();
        book.append_token(&task.task_id, r#"{"replacement":"临近黄昏"}"#)
            .unwrap();
        book.finish_task(&task.task_id, "completed", None, Some("stop"), None)
            .unwrap();
        let polished = book.apply_polish(&task.task_id, false).unwrap();
        assert!(polished.content.starts_with("临近黄昏。母亲"));
        assert_eq!(
            book.apply_polish(&task.task_id, false)
                .err()
                .unwrap()
                .status,
            409
        );
    }

    #[test]
    fn pivot_requires_matching_preview_and_persists_transition() {
        let dir = tempfile::tempdir().unwrap();
        let mut book = Project::create(dir.path().join("路标.story"), "路标").unwrap();
        let initial = book
            .write_arc(Arc {
                arc_id: String::new(),
                title: "旧码头".into(),
                rough_goal: "找到渡船".into(),
                expected_chapters: 9,
                milestones: vec![Milestone {
                    id: "m".into(),
                    text: "问路".into(),
                    done: false,
                }],
                pivot_instruction: String::new(),
                pivot_remaining: 0,
                pivot_start_order: 0,
                status: "active".into(),
                version: 0,
                writer_visible: true,
            })
            .unwrap();
        let first = book.chapters().unwrap().remove(0);
        book.assign_arc(&first.chapter_id, Some(&initial.arc_id))
            .unwrap();
        let preview = book.preview_pivot(&initial.arc_id).unwrap();
        assert_eq!(preview.suggested, 1);
        let change = || Pivot {
            arc_id: initial.arc_id.clone(),
            version: preview.version,
            next_goal: "寻找旧船票".into(),
            instruction: "安排船家带路".into(),
            transition_chapters: 2,
            note_ids: vec![],
        };
        let revised = book.apply_pivot(change()).unwrap();
        assert_eq!(revised.pivot_remaining, 2);
        assert_eq!(revised.pivot_start_order, 1);
        assert_eq!(revised.rough_goal, "寻找旧船票");
        assert_eq!(book.apply_pivot(change()).err().unwrap().status, 409);
        let persisted = Project::open(std::path::PathBuf::from(&book.info.path))
            .unwrap()
            .arcs()
            .unwrap();
        assert_eq!(persisted[0].pivot_instruction, "安排船家带路");
    }
}
