use super::{id, now, Project};
use crate::error::{Result, StudioError};
use rusqlite::{params, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pack {
    pub item_id: String,
    pub item_kind: String,
    pub slot: String,
    pub name: String,
    pub version: String,
    pub priority: i64,
    pub metadata: Value,
    pub body: String,
    pub enabled: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub packs: Vec<Pack>,
    pub active_pack_id: Option<String>,
    pub active_base_skill_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scene {
    pub scene_id: String,
    pub title: String,
    pub version: i64,
    pub details: Value,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SceneInput {
    pub scene_id: Option<String>,
    pub version: Option<i64>,
    pub title: String,
    pub details: Value,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRevision {
    pub prompt_revision_id: String,
    pub instruction: String,
    pub created_at: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trace {
    pub trace_id: String,
    pub task_id: String,
    pub model_profile_id: String,
    pub stage: String,
    pub prompt_tokens: Option<i64>,
    pub cached_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
    pub cost_usd: Option<f64>,
    pub latency_ms: i64,
    pub status: String,
    pub created_at: String,
}

fn json_column(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Value> {
    let text: String = row.get(index)?;
    serde_json::from_str(&text).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(index, rusqlite::types::Type::Text, Box::new(err))
    })
}

impl Project {
    pub fn catalog(&self) -> Result<Catalog> {
        let (active_pack_id, active_base_skill_id) = self.conn.query_row(
            "SELECT active_pack_id,active_base_skill_id FROM project_meta LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let mut query = self.conn.prepare("SELECT item_id,item_kind,slot,name,version,priority,metadata_json,body_markdown,is_enabled
            FROM style_packs ORDER BY item_kind,slot,priority DESC,item_id")?;
        let packs = query
            .query_map([], |r| {
                Ok(Pack {
                    item_id: r.get(0)?,
                    item_kind: r.get(1)?,
                    slot: r.get(2)?,
                    name: r.get(3)?,
                    version: r.get(4)?,
                    priority: r.get(5)?,
                    metadata: json_column(r, 6)?,
                    body: r.get(7)?,
                    enabled: r.get(8)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(Catalog {
            packs,
            active_pack_id,
            active_base_skill_id,
        })
    }

    pub fn write_packs(&mut self, packs: Vec<Pack>, seed: bool) -> Result<Catalog> {
        if packs.len() > 40 {
            return Err(StudioError::new(413, "一次最多写入 40 个配方"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        for pack in packs {
            let valid_slot = match pack.item_kind.as_str() {
                "genre_cartridge" => pack.slot == "cartridge",
                "writing_skill" => matches!(pack.slot.as_str(), "base_style" | "technique"),
                _ => false,
            };
            if !valid_slot
                || pack.item_id.is_empty()
                || pack.item_id.len() > 120
                || pack.name.trim().is_empty()
                || pack.name.chars().count() > 120
                || pack.version.len() > 40
                || !(1..=100).contains(&pack.priority)
                || pack.body.len() > 160_000
                || !pack.metadata.is_object()
            {
                return Err(StudioError::new(400, "配方格式或尺寸有误"));
            }
            let metadata = serde_json::to_string(&pack.metadata)?;
            let exists = tx
                .query_row(
                    "SELECT version,body_markdown,metadata_json FROM style_packs WHERE item_id=?1",
                    [&pack.item_id],
                    |r| {
                        Ok((
                            r.get::<_, String>(0)?,
                            r.get::<_, String>(1)?,
                            r.get::<_, String>(2)?,
                        ))
                    },
                )
                .optional()?;
            if let Some((version, body, saved_metadata)) = exists {
                if seed {
                    continue;
                }
                if version == pack.version && (body != pack.body || saved_metadata != metadata) {
                    return Err(StudioError::new(409, "该配方版本已存在，请先提高版本号"));
                }
            }
            tx.execute("INSERT INTO style_packs(item_id,item_kind,slot,name,version,priority,metadata_json,body_markdown,is_enabled,updated_at)
                VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                ON CONFLICT(item_id) DO UPDATE SET item_kind=excluded.item_kind, slot=excluded.slot,name=excluded.name,
                version=excluded.version,priority=excluded.priority,metadata_json=excluded.metadata_json,
                body_markdown=excluded.body_markdown,updated_at=excluded.updated_at",
                params![pack.item_id,pack.item_kind,pack.slot,pack.name,pack.version,pack.priority,metadata,pack.body,pack.enabled,now()])?;
        }
        tx.commit()?;
        self.catalog()
    }

    pub fn set_pack_enabled(&mut self, id: &str, enabled: bool) -> Result<Catalog> {
        let tx = self.conn.transaction()?;
        tx.execute(
            "UPDATE style_packs SET is_enabled=?1,updated_at=?2 WHERE item_id=?3",
            params![enabled, now(), id],
        )?;
        if !enabled {
            tx.execute("UPDATE project_meta SET active_pack_id=CASE WHEN active_pack_id=?1 THEN NULL ELSE active_pack_id END,
                active_base_skill_id=CASE WHEN active_base_skill_id=?1 THEN NULL ELSE active_base_skill_id END", [id])?;
        }
        tx.commit()?;
        self.catalog()
    }

    pub fn active_style(&self, base: Option<&str>, pack: Option<&str>) -> Result<Catalog> {
        for (id, slot) in [(base, "base_style"), (pack, "cartridge")] {
            if let Some(id) = id {
                let valid: bool = self.conn.query_row("SELECT EXISTS(SELECT 1 FROM style_packs WHERE item_id=?1 AND slot=?2 AND is_enabled=1)",
                    params![id,slot], |r| r.get(0))?;
                if !valid {
                    return Err(StudioError::new(400, "所选底色或卡带不可用"));
                }
            }
        }
        self.conn.execute(
            "UPDATE project_meta SET active_base_skill_id=?1,active_pack_id=?2,updated_at=?3",
            params![base, pack, now()],
        )?;
        self.catalog()
    }

    pub fn scenes(&self) -> Result<Vec<Scene>> {
        let mut query = self.conn.prepare(
            "SELECT scene_id,title,version,scene_json FROM writing_scenes ORDER BY rowid",
        )?;
        let scenes = query
            .query_map([], |r| {
                Ok(Scene {
                    scene_id: r.get(0)?,
                    title: r.get(1)?,
                    version: r.get(2)?,
                    details: json_column(r, 3)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(scenes)
    }

    pub fn save_scene(&self, input: SceneInput) -> Result<Scene> {
        let title = input.title.trim();
        let details = serde_json::to_string(&input.details)?;
        if title.is_empty()
            || title.chars().count() > 120
            || !input.details.is_object()
            || details.len() > 50_000
        {
            return Err(StudioError::new(400, "场景名称或内容无效"));
        }
        let (scene_id, version) = if let Some(scene_id) = input.scene_id {
            let version = input
                .version
                .ok_or_else(|| StudioError::new(409, "缺少场景基线版本"))?;
            let saved = self.conn.execute(
                "UPDATE writing_scenes SET title=?1,scene_json=?2,version=version+1,updated_at=?3
                WHERE scene_id=?4 AND version=?5",
                params![title, details, now(), scene_id, version],
            )?;
            if saved == 0 {
                return Err(StudioError::new(409, "场景已经更新，请刷新后再保存"));
            }
            (scene_id, version + 1)
        } else {
            let scene_id = id();
            self.conn.execute("INSERT INTO writing_scenes(scene_id,title,scene_json,updated_at) VALUES (?1,?2,?3,?4)",
                params![scene_id,title,details,now()])?;
            (scene_id, 1)
        };
        Ok(Scene {
            scene_id,
            title: title.into(),
            version,
            details: input.details,
        })
    }

    pub fn link_scene(&self, chapter: &str, scene: Option<&str>) -> Result<()> {
        let rows = self.conn.execute("UPDATE chapters SET active_scene_id=?1,updated_at=?2 WHERE chapter_id=?3 AND status!='archived'",
            params![scene,now(),chapter])?;
        if rows == 0 {
            return Err(StudioError::new(404, "章节不存在"));
        }
        Ok(())
    }

    pub fn prompt_revisions(&self) -> Result<Vec<PromptRevision>> {
        let mut query = self.conn.prepare("SELECT prompt_revision_id,instruction,created_at FROM prompt_revisions ORDER BY rowid DESC")?;
        let rows = query
            .query_map([], |r| {
                Ok(PromptRevision {
                    prompt_revision_id: r.get(0)?,
                    instruction: r.get(1)?,
                    created_at: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    pub fn save_prompt_revision(&self, instruction: &str) -> Result<PromptRevision> {
        if instruction.len() > 16_000 {
            return Err(StudioError::new(413, "提示词覆写超出长度上限"));
        }
        let row = PromptRevision {
            prompt_revision_id: id(),
            instruction: instruction.trim().into(),
            created_at: now(),
        };
        self.conn.execute("INSERT INTO prompt_revisions(prompt_revision_id,instruction,created_at) VALUES (?1,?2,?3)",
            params![row.prompt_revision_id,row.instruction,row.created_at])?;
        Ok(row)
    }

    pub fn traces(&self) -> Result<Vec<Trace>> {
        let mut query = self.conn.prepare("SELECT trace_id,task_id,model_profile_id,stage,prompt_tokens,cached_tokens,completion_tokens,cost_usd,latency_ms,status,created_at
            FROM llm_traces ORDER BY rowid DESC")?;
        let rows = query
            .query_map([], |r| {
                Ok(Trace {
                    trace_id: r.get(0)?,
                    task_id: r.get(1)?,
                    model_profile_id: r.get(2)?,
                    stage: r.get(3)?,
                    prompt_tokens: r.get(4)?,
                    cached_tokens: r.get(5)?,
                    completion_tokens: r.get(6)?,
                    cost_usd: r.get(7)?,
                    latency_ms: r.get(8)?,
                    status: r.get(9)?,
                    created_at: r.get(10)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }
}
