use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::{current_revision, find_task, id, now, Project, Revision, RevisionInput, Task};
use crate::{
    error::{Result, StudioError},
    profiles::Profile,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    pub task_id: String,
    pub reserved_usd: f64,
    pub adopted_revision_id: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Batch {
    pub batch_id: String,
    pub chapter_ids: Vec<String>,
    pub cursor: usize,
    pub phase: String,
    pub audited_revision_id: Option<String>,
    pub state: String,
    pub budget_usd: f64,
    pub spent_usd: f64,
    pub instruction: String,
    pub writer_profile: String,
    pub audit_profile: String,
    pub extraction_profile: String,
    pub reason: String,
    pub estimated_steps: i64,
    pub step: Option<Step>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchInput {
    pub chapter_ids: Vec<String>,
    pub budget_usd: f64,
    pub instruction: String,
    pub writer_profile: String,
    pub audit_profile: String,
    pub extraction_profile: String,
}

fn valid_budget(budget: f64) -> bool {
    budget.is_finite() && (0.000_001..=10_000.0).contains(&budget)
}
fn conflict(message: &str) -> StudioError {
    StudioError::new(409, message)
}

fn batch(db: &Connection, requested: Option<&str>) -> Result<Option<Batch>> {
    let stored = db.query_row("SELECT batch_id,chapters_json,cursor,phase,state,budget_usd,spent_usd,instruction,writer_profile,
        audit_profile,extraction_profile,reason,audited_revision_id FROM batch_runs WHERE (?1 IS NULL OR batch_id=?1) ORDER BY rowid DESC LIMIT 1",
        [requested], |r| Ok((r.get::<_,String>(1)?, Batch {
            batch_id:r.get(0)?,chapter_ids:vec![],cursor:r.get(2)?,phase:r.get(3)?,state:r.get(4)?,budget_usd:r.get(5)?,
            spent_usd:r.get(6)?,instruction:r.get(7)?,writer_profile:r.get(8)?,audit_profile:r.get(9)?,
            extraction_profile:r.get(10)?,reason:r.get(11)?,audited_revision_id:r.get(12)?,estimated_steps:0,step:None,
        }))).optional()?;
    let Some((chapters, mut run)) = stored else {
        return Ok(None);
    };
    run.chapter_ids = serde_json::from_str(&chapters)?;
    run.step = db.query_row("SELECT task_id,reserved_usd,adopted_revision_id FROM batch_steps WHERE batch_id=?1 AND settled_usd IS NULL",
        [&run.batch_id], |r| Ok(Step{task_id:r.get(0)?,reserved_usd:r.get(1)?,adopted_revision_id:r.get(2)?})).optional()?;
    run.estimated_steps = db.query_row("SELECT COUNT(*) FROM batch_steps s JOIN generation_tasks g ON s.task_id=g.task_id
        LEFT JOIN llm_traces l ON l.task_id=s.task_id WHERE s.batch_id=?1 AND s.settled_usd IS NOT NULL AND l.cost_usd IS NULL
        AND g.state!='queued'", [&run.batch_id], |r| r.get(0))?;
    Ok(Some(run))
}

fn required(db: &Connection, id: &str) -> Result<Batch> {
    batch(db, Some(id))?.ok_or_else(|| conflict("后台批次已变化，请刷新后重试"))
}

fn update_pause(db: &Connection, id: &str, reason: &str) -> Result<()> {
    db.execute("UPDATE batch_runs SET state='paused',reason=?1 WHERE batch_id=?2 AND state IN('running','paused')",params![reason,id])?;
    Ok(())
}

pub fn profile_price(profile: &Profile) -> Option<(f64, f64)> {
    let rates = profile.rates.as_ref()?;
    let input = rates
        .input
        .max(rates.cache_write.unwrap_or(rates.input))
        .max(rates.cache_read.unwrap_or(rates.input));
    (input.is_finite() && rates.output.is_finite() && input >= 0.0 && rates.output >= 0.0)
        .then_some((input, rates.output))
}

impl Project {
    pub fn latest_batch(&self) -> Result<Option<Batch>> {
        batch(&self.conn, None)
    }

    pub fn start_batch(&mut self, input: BatchInput) -> Result<Batch> {
        if !valid_budget(input.budget_usd)
            || input.chapter_ids.is_empty()
            || input.chapter_ids.len() > 100
            || input.instruction.len() > 4800
        {
            return Err(StudioError::new(400, "请选择 1 至 100 章并填写有效预算"));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if batch(&tx, None)?.is_some_and(|run| matches!(run.state.as_str(), "running" | "paused")) {
            return Err(conflict("请完成或结束已有后台批次"));
        }
        let mut chapters = self_order_chapters(&tx, &input.chapter_ids)?;
        if chapters.len() != input.chapter_ids.len() {
            return Err(conflict("章序已有变化或章节已归档"));
        }
        chapters.sort_by_key(|row| row.0);
        let ids: Vec<_> = chapters.into_iter().map(|(_, id)| id).collect();
        let batch_id = id();
        tx.execute("INSERT INTO batch_runs(batch_id,chapters_json,budget_usd,instruction,writer_profile,audit_profile,extraction_profile,created_at)
            VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",params![batch_id,serde_json::to_string(&ids)?,input.budget_usd,
                input.instruction,input.writer_profile,input.audit_profile,input.extraction_profile,now()])?;
        let run = required(&tx, &batch_id)?;
        tx.commit()?;
        Ok(run)
    }

    pub fn control_batch(
        &self,
        id: &str,
        action: &str,
        budget_usd: Option<f64>,
        reason: Option<&str>,
    ) -> Result<Batch> {
        let run = required(&self.conn, id)?;
        match action {
            "pause" if run.state == "running" => {
                update_pause(&self.conn, id, reason.unwrap_or("作者暂停"))?
            }
            "stop" if matches!(run.state.as_str(), "paused" | "running") && run.step.is_none() => {
                self.conn.execute(
                    "UPDATE batch_runs SET state='stopped',reason='作者结束批次' WHERE batch_id=?1",
                    [id],
                )?;
            }
            "resume" if run.state == "paused" => {
                let budget = budget_usd.unwrap_or(run.budget_usd);
                if !valid_budget(budget)
                    || budget + 1.0e-12
                        < run.spent_usd + run.step.as_ref().map_or(0.0, |step| step.reserved_usd)
                {
                    return Err(StudioError::new(400, "预算必须覆盖已记账和预留用量"));
                }
                self.conn.execute("UPDATE batch_runs SET state='running',budget_usd=?1,reason='' WHERE batch_id=?2",params![budget,id])?;
            }
            _ => return Err(conflict("批次状态已更新。等待当前请求结算后可结束。")),
        }
        required(&self.conn, id)
    }

    pub fn reserve_batch_step(
        &mut self,
        id: &str,
        task_id: &str,
        profile: &Profile,
    ) -> Result<Batch> {
        let (input_rate, output_rate) = profile_price(profile)
            .ok_or_else(|| StudioError::new(400, "后台任务需要配置模型单价"))?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = required(&tx, id)?;
        let task = find_task(&tx, task_id)?;
        let (kind, profile_id) = match run.phase.as_str() {
            "write" => ("batch_write", &run.writer_profile),
            "audit" => ("consistency_check", &run.audit_profile),
            "extract" => ("memory_extract", &run.extraction_profile),
            _ => return Err(conflict("此阶段无需付费生成")),
        };
        if run.state != "running"
            || run.step.is_some()
            || task.state != "queued"
            || task.task_type != kind
            || task.frozen_context["batchId"].as_str() != Some(id)
            || task.frozen_context["profileId"].as_str() != Some(profile_id)
            || profile_id != &profile.id
            || run.chapter_ids.get(run.cursor) != Some(&task.chapter_id)
            || task.base_revision_id != current_revision(&tx, &task.chapter_id)?
        {
            return Err(conflict("后台任务状态有变化，请刷新批次"));
        }
        let body = &task.frozen_context["requestBody"];
        let Some(max_output) = body[&profile.token_limit_field]
            .as_u64()
            .filter(|n| (1..=8192).contains(n))
        else {
            return Err(StudioError::new(400, "此模型未声明可验证的输出 token 上限"));
        };
        let prompt_len = task.frozen_context["prompt"]
            .as_str()
            .unwrap_or_default()
            .len();
        if prompt_len > 180_000 {
            update_pause(
                &tx,
                id,
                "本次上下文超过已配置计费区间，请缩短正文或改用手动审查",
            )?;
        } else {
            let estimate =
                ((prompt_len + 512) as f64 * input_rate + max_output as f64 * output_rate) / 1e6;
            if run.spent_usd + estimate > run.budget_usd + 1.0e-12 {
                update_pause(&tx, id, "剩余预算不足以预留下一次请求")?;
            } else {
                tx.execute("INSERT INTO batch_steps(task_id,batch_id,chapter_id,phase,reserved_usd,created_at) VALUES(?1,?2,?3,?4,?5,?6)",
                    params![task_id,id,task.chapter_id,run.phase,estimate,now()])?;
            }
        }
        let next = required(&tx, id)?;
        tx.commit()?;
        Ok(next)
    }

    pub fn check_batch_dispatch(&self, task: &Task) -> Result<()> {
        let id = task.frozen_context["batchId"].as_str();
        if id.is_none() && task.task_type != "batch_write" {
            return Ok(());
        }
        let run = required(
            &self.conn,
            id.ok_or_else(|| conflict("后台请求未登记预算"))?,
        )?;
        if run.state != "running" || run.step.is_none_or(|step| step.task_id != task.task_id) {
            return Err(conflict("后台请求尚未预留预算，或批次已暂停"));
        }
        Ok(())
    }

    pub fn adopt_batch_draft(&mut self, batch_id: &str, task_id: &str) -> Result<Revision> {
        let run = required(&self.conn, batch_id)?;
        let step = run
            .step
            .filter(|step| step.task_id == task_id)
            .ok_or_else(|| conflict("该稿不在当前批次"))?;
        let task = find_task(&self.conn, task_id)?;
        if task.task_type != "batch_write" || task.state != "completed" {
            return Err(conflict("该稿尚不能采纳"));
        }
        if let Some(adopted) = step.adopted_revision_id {
            return self
                .revisions(&task.chapter_id)?
                .into_iter()
                .find(|revision| revision.revision_id == adopted)
                .ok_or_else(|| conflict("找不到已采用修订"));
        }
        self.save_revision(RevisionInput {
            chapter_id: task.chapter_id,
            base_revision_id: task.base_revision_id,
            content: String::new(),
            source: "ai_gen".into(),
            draft_task_id: Some(task_id.into()),
            edit_level: None,
            canon_confirmed: false,
        })
    }

    pub fn settle_batch_step(
        &mut self,
        id: &str,
        task_id: &str,
        problem: &str,
        passed: bool,
    ) -> Result<Batch> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = required(&tx, id)?;
        let Some(step) = run.step.filter(|step| step.task_id == task_id) else {
            return required(&tx, id);
        };
        let task = find_task(&tx, task_id)?;
        if matches!(task.state.as_str(), "queued" | "running" | "streaming") {
            return Err(conflict("模型请求仍在运行"));
        }
        let usage = tx
            .query_row(
                "SELECT cost_usd,prompt_tokens,completion_tokens,cached_tokens FROM llm_traces
            WHERE task_id=?1 ORDER BY rowid DESC LIMIT 1",
                [task_id],
                |r| {
                    Ok((
                        r.get::<_, Option<f64>>(0)?,
                        r.get::<_, Option<i64>>(1)?,
                        r.get::<_, Option<i64>>(2)?,
                        r.get::<_, Option<i64>>(3)?,
                    ))
                },
            )
            .optional()?;
        let estimated_rates = &task.frozen_context["rates"];
        let known_billed = usage.and_then(|(cost, prompt, completion, cached)| {
            cost.or_else(|| {
                let input = estimated_rates["input"].as_f64()?;
                let output = estimated_rates["output"].as_f64()?;
                let cache_read = estimated_rates["cacheRead"].as_f64().unwrap_or(input);
                let max_fill = input.max(estimated_rates["cacheWrite"].as_f64().unwrap_or(input));
                let prompt = prompt?;
                let completion = completion?;
                let count = cached.unwrap_or(0);
                if prompt < count || count < 0 || completion < 0 {
                    return None;
                }
                let rate = if cached.is_some() {
                    max_fill
                } else {
                    max_fill.max(cache_read)
                };
                Some(
                    ((prompt - count) as f64 * rate
                        + count as f64 * cache_read
                        + completion as f64 * output)
                        / 1e6,
                )
            })
        });
        let sent: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM task_events WHERE task_id=?1
            AND json_extract(payload_json,'$.state') IN('running','streaming'))",
            [task_id],
            |r| r.get(0),
        )?;
        let amount = known_billed.unwrap_or(if sent { step.reserved_usd } else { 0.0 });
        let next = if task.state == "completed" && passed {
            match run.phase.as_str() {
                "write" if step.adopted_revision_id.is_some() => {
                    let chars: i64 = tx.query_row(
                        "SELECT cjk_char_count FROM chapter_revisions WHERE revision_id=?1",
                        [step.adopted_revision_id],
                        |r| r.get(0),
                    )?;
                    if chars >= self.info.target_chapter_chars {
                        "audit"
                    } else {
                        "write"
                    }
                }
                "audit" => "extract",
                "extract" => "advance",
                _ => run.phase.as_str(),
            }
        } else {
            run.phase.as_str()
        };
        tx.execute(
            "UPDATE batch_steps SET settled_usd=?1 WHERE task_id=?2",
            params![amount, task_id],
        )?;
        tx.execute(
            "UPDATE batch_runs SET spent_usd=spent_usd+?1,phase=?2 WHERE batch_id=?3",
            params![amount, next, id],
        )?;
        if run.phase == "audit" && next == "extract" {
            tx.execute(
                "UPDATE batch_runs SET audited_revision_id=?1 WHERE batch_id=?2",
                params![task.base_revision_id, id],
            )?;
        }
        let reason = if matches!(task.state.as_str(), "outcome_unknown" | "interrupted") {
            "请求结果不明，已按可用用量或预留上限记账"
        } else if sent && known_billed.is_none() {
            "提供方未返回完整用量，已预留本次可能发生的费用"
        } else if run.spent_usd + amount > run.budget_usd {
            "供应商返回的实际用量超过预留预算"
        } else if !problem.is_empty() {
            problem
        } else if task.state != "completed" || !passed {
            "后台步骤没有完成"
        } else {
            ""
        };
        if !reason.is_empty() {
            update_pause(&tx, id, reason)?;
        }
        let updated = required(&tx, id)?;
        tx.commit()?;
        Ok(updated)
    }

    pub fn advance_batch(&mut self, id: &str) -> Result<Batch> {
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let run = required(&tx, id)?;
        if run.state != "running" || run.step.is_some() {
            return Err(conflict("批次尚有未完成步骤"));
        }
        let chapter = run
            .chapter_ids
            .get(run.cursor)
            .ok_or_else(|| conflict("批次已经结束"))?;
        let (state,count,current): (String,i64,Option<String>) = tx.query_row("SELECT c.memory_sync_status,COALESCE(r.cjk_char_count,0),c.current_revision_id FROM chapters c
            LEFT JOIN chapter_revisions r ON r.revision_id=c.current_revision_id WHERE c.chapter_id=?1 AND c.status!='archived'",
            [chapter],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?;
        match run.phase.as_str() {
            "write" if count >= self.info.target_chapter_chars => {
                tx.execute(
                    "UPDATE batch_runs SET phase='audit' WHERE batch_id=?1",
                    [id],
                )?;
            }
            "extract" | "advance" if current != run.audited_revision_id => {
                tx.execute("UPDATE batch_runs SET phase='audit',audited_revision_id=NULL WHERE batch_id=?1",[id])?;
            }
            "advance" if state == "synced" => {
                tx.execute("UPDATE batch_runs SET cursor=cursor+1,phase='write',state=?1,audited_revision_id=NULL WHERE batch_id=?2",
                    params![if run.cursor+1>=run.chapter_ids.len() {"completed"} else {"running"},id])?;
            }
            "advance" => update_pause(&tx, id, "记忆中仍有候选或缺失证据，请在审核后继续")?,
            _ => return Err(conflict("当前步骤尚不能推进")),
        }
        let updated = required(&tx, id)?;
        tx.commit()?;
        Ok(updated)
    }
}

fn self_order_chapters(db: &Connection, ids: &[String]) -> Result<Vec<(i64, String)>> {
    let mut query = db.prepare(
        "SELECT chapter_order,chapter_id FROM chapters WHERE status!='archived'
        AND chapter_id IN (SELECT value FROM json_each(?1))",
    )?;
    let rows = query
        .query_map([serde_json::to_string(ids)?], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{Metering, TaskInput};
    use crate::profiles::{Capabilities, Rates};
    use serde_json::json;

    fn profile() -> Profile {
        Profile {
            id: "fixture".into(),
            version: "one".into(),
            name: "本地验证".into(),
            base_url: "http://127.0.0.1/".into(),
            model: "test".into(),
            credential_ref: String::new(),
            needs_key: false,
            direct: true,
            token_limit_field: "max_tokens".into(),
            capabilities: Capabilities::default(),
            rates: Some(Rates {
                input: 1.0,
                output: 1.0,
                cache_read: None,
                cache_write: None,
            }),
        }
    }

    fn run(chapters: Vec<String>, budget: f64) -> BatchInput {
        BatchInput {
            chapter_ids: chapters,
            budget_usd: budget,
            instruction: String::new(),
            writer_profile: "fixture".into(),
            audit_profile: "fixture".into(),
            extraction_profile: "fixture".into(),
        }
    }
    fn create(project: &mut Project, run: &Batch, kind: &str, base: Option<String>) -> Task {
        let chapter_id = run.chapter_ids[run.cursor].clone();
        project.create_task(TaskInput { idempotency_key:id(),chapter_id,base_revision_id:base,task_type:kind.into(),
            frozen_context:json!({"batchId":run.batch_id,"profileId":"fixture","prompt":"测试","rates":{"input":1,"output":1},
                "requestBody":{"max_tokens":4096}}) }).unwrap()
    }

    #[test]
    fn batch_reservations_pause_on_budget_disconnect_and_severe_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("批次.story");
        let mut project = Project::create(path.clone(), "批次验收").unwrap();
        let chapter = project.chapters().unwrap().remove(0).chapter_id;
        let initial = project
            .start_batch(run(vec![chapter.clone()], 0.00001))
            .unwrap();
        let candidate = create(&mut project, &initial, "batch_write", None);
        assert_eq!(
            project.check_batch_dispatch(&candidate).unwrap_err().status,
            409
        );
        let exhausted = project
            .reserve_batch_step(&initial.batch_id, &candidate.task_id, &profile())
            .unwrap();
        assert_eq!(exhausted.state, "paused");
        assert!(exhausted.step.is_none());
        let resumed = project
            .control_batch(&initial.batch_id, "resume", Some(1.0), None)
            .unwrap();
        let reserved = project
            .reserve_batch_step(&resumed.batch_id, &candidate.task_id, &profile())
            .unwrap();
        let max_charge = reserved.step.unwrap().reserved_usd;
        assert!(max_charge > 0.0);
        project.check_batch_dispatch(&candidate).unwrap();
        project.claim_task(&candidate.task_id).unwrap();
        project
            .finish_task(&candidate.task_id, "outcome_unknown", None, None, None)
            .unwrap();
        let unknown = project
            .settle_batch_step(&resumed.batch_id, &candidate.task_id, "", false)
            .unwrap();
        assert_eq!(unknown.state, "paused");
        assert!((unknown.spent_usd - max_charge).abs() < 1e-12);
        assert_eq!(
            project
                .settle_batch_step(&resumed.batch_id, &candidate.task_id, "", false)
                .unwrap()
                .spent_usd,
            unknown.spent_usd
        );
        let mut project = Project::open(path).unwrap();
        assert_eq!(project.latest_batch().unwrap().unwrap().state, "paused");
        let resumed = project
            .control_batch(&initial.batch_id, "resume", None, None)
            .unwrap();
        let draft = create(&mut project, &resumed, "batch_write", None);
        project
            .reserve_batch_step(&resumed.batch_id, &draft.task_id, &profile())
            .unwrap();
        project.claim_task(&draft.task_id).unwrap();
        project
            .append_token(&draft.task_id, &"雨下在空船上。".repeat(400))
            .unwrap();
        let usage = || {
            Some(Metering {
                cost_usd: Some(0.0003),
                ..Default::default()
            })
        };
        project
            .finish_task(&draft.task_id, "completed", None, None, usage())
            .unwrap();
        let accepted = project
            .adopt_batch_draft(&resumed.batch_id, &draft.task_id)
            .unwrap();
        assert_eq!(
            project
                .adopt_batch_draft(&resumed.batch_id, &draft.task_id)
                .unwrap()
                .revision_id,
            accepted.revision_id
        );
        let audit_stage = project
            .settle_batch_step(&resumed.batch_id, &draft.task_id, "", true)
            .unwrap();
        assert_eq!(audit_stage.phase, "audit");
        let audit = create(
            &mut project,
            &audit_stage,
            "consistency_check",
            Some(accepted.revision_id.clone()),
        );
        project
            .reserve_batch_step(&initial.batch_id, &audit.task_id, &profile())
            .unwrap();
        project.claim_task(&audit.task_id).unwrap();
        project
            .finish_task(&audit.task_id, "completed", None, None, usage())
            .unwrap();
        let hold = project
            .settle_batch_step(&initial.batch_id, &audit.task_id, "重大一致性告警", true)
            .unwrap();
        assert_eq!(hold.state, "paused");
        assert_eq!(hold.phase, "extract");
        assert_eq!(hold.audited_revision_id, Some(accepted.revision_id));
    }
}
