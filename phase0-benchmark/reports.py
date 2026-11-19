"""Blind review materials, private billing and evidence-based calibration."""

from collections import Counter, defaultdict
from contextlib import closing
import csv
import json
from pathlib import Path
import re
import secrets
import sqlite3
from statistics import mean, median
from urllib.parse import urlsplit

from benchmark_core import DIMENSIONS, require


USAGE_FIELDS = ("prompt_tokens", "cached_tokens", "completion_tokens", "cost_usd", "latency_ms")
RATING_FIELDS = ("sample", *DIMENSIONS, "pov_leak", "evidence")
SKILL_FIELDS = ("example", "score", "evidence")


def load_run(directory):
    require((directory / "ledger.sqlite3").is_file(), "运行目录缺少 ledger.sqlite3")
    with closing(sqlite3.connect(directory / "ledger.sqlite3")) as db:
        run_start = db.execute("SELECT frozen_json FROM run_info").fetchone()
        require(run_start, "运行目录缺少冻结输入")
        manifest = json.loads(run_start[0])
        items = []
        for call_id, recorded_state, source, result in db.execute(
            "SELECT call_id, state, input_json, result_json FROM calls ORDER BY call_id"
        ):
            state = "outcome_unknown" if recorded_state == "running" else recorded_state
            items.append({"call_id": call_id, **json.loads(source), **(json.loads(result) if result else {}), "state": state})
    return manifest, items


def scrubber(manifest, rows):
    identities = {profile["model"] for profile in manifest["profiles"]}
    identities.update(urlsplit(profile["base_url"]).hostname for profile in manifest["profiles"])
    identities.update(row["reported_model"] for row in rows if row.get("reported_model"))
    pattern = re.compile("|".join(re.escape(name) for name in sorted(identities, key=len, reverse=True) if name), re.IGNORECASE)
    return lambda value: pattern.sub("[已隐去]", value)


def quote_markdown(text):
    # Prevent provider text from adding headings, links or embedded content to the report.
    return "\n".join("> " + re.sub(r"([\\`*_{}\[\]<>()#!|])", r"\\\1", part) for part in text.splitlines())


def csv_file(path, rows, fields, new_only=False):
    if new_only and path.exists():
        return
    with path.open("w", encoding="utf-8-sig", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)


def reviewable(row):
    return row["role"] == "主笔" and bool(row.get("content"))


def skill_examples(skills):
    for skill in skills.values():
        for index, example in enumerate(re.split(r"\n\s*\n", skill["body"].split("## 正例\n", 1)[1]
                                                 .split("## 反例\n", 1)[0].strip()), 1):
            yield {"example": f"{skill['id']}@{skill['version']}:p{index}", "kind": "正例", "text": example}
        for index, example in enumerate(re.split(r"\n\s*\n", skill["body"].split("## 反例\n", 1)[1].strip()), 1):
            yield {"example": f"{skill['id']}@{skill['version']}:n{index}", "kind": "反例", "text": example}


def report(directory):
    manifest, rows = load_run(directory)
    redact = scrubber(manifest, rows)
    blind = [row for row in rows if reviewable(row)]
    scenarios = {s["id"]: s for s in manifest["scenarios"]}
    groups = defaultdict(list)
    for row in blind:
        groups[(row["scenario"], row["beat"])].append(row)
    lines = [
        "# StoryForge V3 匿名正文盲评",
        "",
        "对照同组故事条件，按 rubric.md 给每个编号独立评分。保留样本编号并填写 ratings-template.csv。",
        f"本轮计划写作 {sum(row['role'] == '主笔' for row in rows)} 次，当前可供阅读 {len(blind)} 份。",
    ]
    for (scenario_id, beat_no), group in sorted(groups.items()):
        scenario = scenarios[scenario_id]
        beat = scenario["beats"][beat_no - 1]
        lines.extend(["", f"## {scenario_id} {scenario['title']} · 节拍 {beat_no}", "", f"视角人物 {beat['pov']}"])
        clues = [clue for fact in scenario.get("facts", []) for clue in fact["observable_clues"]]
        lines.extend(["", quote_markdown("\n".join([scenario["public"], beat["context"], beat["goal"], beat["tone"], *clues])),
                      "", "评审用知情边界仅供评审对照。"])
        for fact in scenario.get("facts", []):
            view = fact["views"].get(beat["pov"], {})
            state = view.get("state", "未知")
            lines.append(quote_markdown(f"{fact['truth']} · {state} · {view.get('text', '')}"))
        if not scenario.get("facts"):
            lines.append("本拍未设置私有事实。")
        for row in sorted(group, key=lambda item: item["score_id"]):
            lines.extend(["", f"### 样本 {row['score_id']}", "", quote_markdown(redact(row["content"]))])
    (directory / "blind-review.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    (directory / "rubric.md").write_bytes(Path(__file__).with_name("rubric.md").read_bytes())
    csv_file(directory / "ratings-template.csv", ({"sample": row["score_id"]} for row in sorted(blind, key=lambda r: r["score_id"])),
             RATING_FIELDS, new_only=True)
    examples = list(skill_examples(manifest["skills"]))
    skill_lines = ["# Skill 正反例评审", "", "参考 rubric.md，针对每段正反例填写独立评分与证据。"]
    for example in examples:
        skill_lines.extend(["", f"## {example['example']} · {example['kind']}", "", example["text"]])
    (directory / "skill-review.md").write_text("\n".join(skill_lines) + "\n", encoding="utf-8")
    csv_file(directory / "skills-template.csv", examples, SKILL_FIELDS, new_only=True)
    private = directory / "private"
    private.mkdir(exist_ok=True)
    csv_file(private / "usage.csv", rows, ("call_id", "profile_id", "role", "scenario", "beat", "method", "sample",
                                           "band", "case_id", "score_id", "state", "error", "finish_reason", *USAGE_FIELDS,
                                           "cjk_chars", "writing_usable", "leak_markers", "role_pass"))
    totals = {}
    for profile in manifest["profiles"]:
        profile_rows = [row for row in rows if row["profile_id"] == profile["id"]]
        totals[profile["id"]] = {
            "model": profile["model"],
            "states": Counter(row["state"] for row in profile_rows),
            **{field: {"observed_sum": sum(row[field] for row in profile_rows if row.get(field) is not None),
                       "known_count": sum(row.get(field) is not None for row in profile_rows)} for field in USAGE_FIELDS},
        }
    (private / "totals.json").write_text(json.dumps(totals, ensure_ascii=False, indent=2), encoding="utf-8")
    ledger = [row for row in rows if row.get("scenario") == "03" and row.get("method") == "v3"]
    fixture_issues = [row["score_id"] for row in ledger if row.get("leak_markers")]
    remaining = [row for row in ledger if row["state"] != "completed"]
    summary = {"frozen_inputs_sha256": manifest["input_sha256"], "states": Counter(row["state"] for row in rows),
               "ledger_pov_marker_hits": fixture_issues, "ledger_pov_outputs_missing": len(remaining),
               "human_review": "pending"}
    (private / "run-status.json").write_text(json.dumps(summary, ensure_ascii=False, indent=2), encoding="utf-8")


def read_ratings(paths, fields, allowed_ids, evidence_for):
    ratings = defaultdict(list)
    for path in paths:
        seen = set()
        with path.open(encoding="utf-8-sig", newline="") as stream:
            reader = csv.DictReader(stream)
            require(tuple(reader.fieldnames or ()) == fields, f"{path.name} 列名与模板不符")
            for line, row in enumerate(reader, 2):
                location = f"{path.name}:{line}"
                require(None not in row and all(isinstance(row[key], str) for key in fields), f"{location} 列数不符")
                identifier = row[fields[0]].strip()
                require(identifier in allowed_ids and identifier not in seen, f"{location} 未知或重复的编号")
                seen.add(identifier)
                if not any(row[key].strip() for key in fields[1:]):
                    continue
                keys = DIMENSIONS if fields == RATING_FIELDS else ("score",)
                require(all(row[key].strip() in "12345" and len(row[key].strip()) == 1 for key in keys), f"{location} 分数须为 1 至 5")
                scores = {key: int(row[key]) for key in keys}
                if fields == RATING_FIELDS:
                    require(row["pov_leak"].strip() in {"yes", "no"}, f"{location} pov_leak 须填写 yes 或 no")
                    scores["pov_leak"] = row["pov_leak"].strip() == "yes"
                if evidence_for(scores):
                    require(row["evidence"].strip(), f"{location} 请填写原文证据")
                ratings[identifier].append(scores)
    return ratings


def average_cost(rows):
    values = [row.get("cost_usd") for row in rows]
    return mean(values) if values and all(value is not None for value in values) else None


def calibrate(directory, rating_paths, skill_paths):
    manifest, rows = load_run(directory)
    profile_by_id = {p["id"]: p for p in manifest["profiles"]}
    writing = [row for row in rows if row["role"] == "主笔"]
    examples = list(skill_examples(manifest["skills"]))
    require(len({path.resolve() for path in rating_paths}) == len(rating_paths), "正文评分文件重复")
    require(len({path.resolve() for path in skill_paths}) == len(skill_paths), "Skill 评分文件重复")
    ratings = read_ratings(rating_paths, RATING_FIELDS, {r["score_id"] for r in writing if reviewable(r)},
                           lambda score: score["pov_leak"] or min(score[d] for d in DIMENSIONS) <= 2)
    skill_ratings = read_ratings(skill_paths, SKILL_FIELDS, {e["example"] for e in examples}, lambda _: True)
    groups = defaultdict(list)
    role_groups = defaultdict(list)
    for row in rows:
        if row["role"] != "主笔":
            role_groups[(row["profile_id"], row["role"])].append(row)
            continue
        review = ratings.get(row["score_id"], [])
        row["mean_rating"] = mean(mean(score[d] for d in DIMENSIONS) for score in review) if review else None
        row["human_leak"] = any(score["pov_leak"] for score in review)
        row["pass"] = bool(row.get("writing_usable") and review and row["mean_rating"] >= 4
                           and not row["human_leak"] and not row.get("leak_markers")
                           and all(mean(score[d] for score in review) >= 4 for d in ("pov", "continuity")))
        groups[(row["profile_id"], row["method"], *row["band"])].append(row)
    failed = [row for row in rows if row["state"] != "completed"]
    usage_missing = [row for row in rows if row.get("prompt_tokens") is None or row.get("completion_tokens") is None]
    unreviewed = [row for row in writing if not ratings.get(row["score_id"])]
    pov_fails = [row for row in writing if row["scenario"] == "03" and row["method"] == "v3"
                 and (row["human_leak"] or row.get("leak_markers"))]
    unreviewed_skills = [example for example in examples if not skill_ratings.get(example["example"])]
    lines = ["# StoryForge V3 Phase 0 校准", "", f"冻结输入 SHA-256 `{manifest['input_sha256']}`", "",
             "| 检查 | 未满足数 |", "| --- | ---: |",
             f"| 调用完成 | {len(failed)} |", f"| 输入和输出 token 由供应商返回 | {len(usage_missing)} |",
             f"| 费用可估算 | {sum(row.get('cost_usd') is None for row in rows)} |",
             f"| 正文有评分 | {len(unreviewed)} |", f"| 场景 03 V3 已知泄密 | {len(pov_fails)} |",
             f"| 正反例有评分 | {len(unreviewed_skills)} |", "", "## 按模型与参考篇幅", "",
             "| 配置 | 方法 | 参考字数 | 通过 / 全部 | 通过样本实际字数范围 | 实际汉字中位数 | 盲评分均值 | 单次估算 USD |",
             "| --- | --- | --- | ---: | ---: | ---: | ---: | ---: |"]
    stats = {}
    for (profile, method, low, high), group in sorted(groups.items()):
        assessed = [row["mean_rating"] for row in group if row["mean_rating"] is not None]
        counts = [row["cjk_chars"] for row in group if row.get("writing_usable")]
        accepted_counts = [row["cjk_chars"] for row in group if row["pass"]]
        actual_range = f"{min(accepted_counts)}–{max(accepted_counts)}" if accepted_counts else "未知"
        passed = sum(row["pass"] for row in group)
        cost = average_cost(group)
        stats[(profile, method, low, high)] = (passed, len(group), len(assessed), mean(assessed) if assessed else 0, cost)
        lines.append(f"| {profile} | {method} | {low}–{high} | {passed}/{len(group)} | {actual_range} | "
                     f"{median(counts) if counts else '未知'} | {fmt(mean(assessed) if assessed else None, 2)} | {fmt(cost)} |")
    lines.extend(["", "### 篇幅候选", ""])
    options = []
    length_calibrated = 0
    for profile in profile_by_id:
        eligible = [(passed / total, score, low, high, cost)
                    for (identifier, method, low, high), (passed, total, rated, score, cost) in stats.items()
                    if identifier == profile and method == "v3" and rated == total and passed > 0]
        if eligible:
            length_calibrated += 1
            rate, score, low, high, cost = max(eligible, key=lambda item: (item[0], item[1], -abs(item[3] - item[2])))
            lines.append(f"- {profile} · {low}–{high} 汉字 · 通过率 {rate:.0%} · 均分 {score:.2f}")
            if rate >= 0.8 and cost is not None:
                options.append((profile, cost))
        else:
            lines.append(f"- {profile} · 待取得完整评分与通过样本")
    lines.extend(["", "## 审查与提取", "", "| 角色 | 配置 | 合格 / 全部 | 中位耗时 ms | 单次估算 USD |",
                  "| --- | --- | ---: | ---: | ---: |"])
    candidates = defaultdict(list)
    for (profile, role), group in sorted(role_groups.items()):
        cost = average_cost(group)
        durations = [row["latency_ms"] for row in group if row.get("latency_ms") is not None]
        passed = sum(bool(row.get("role_pass")) for row in group)
        latency = median(durations) if durations else None
        lines.append(f"| {role} | {profile} | {passed}/{len(group)} | {latency if latency is not None else '未知'} | {fmt(cost)} |")
        if passed == len(group) and cost is not None:
            candidates[role].append((cost, latency, profile))
    lines.append("")
    routes_ready = bool(options and candidates["审查"] and candidates["提取"]
                        and not (failed or usage_missing or unreviewed or pov_fails))
    if routes_ready:
        writer, writing_cost = min(options, key=lambda item: item[1])
        audit_cost, _, audit = min(candidates["审查"])
        memory_cost, _, memory = min(candidates["提取"])
        lines.append(f"成本候选组合为主笔 {writer}、审查 {audit}、提取 {memory}。按每角色各调用一次估算 "
                     f"{fmt(writing_cost + audit_cost + memory_cost)} USD。主笔通过率至少 80%，审查和提取须通过全部题例。")
    else:
        lines.append("待收齐完整结果、人工评分与费用数据，并取得满足门槛的三角色配置。")
    lines.extend(["", "## 首批正反例", ""])
    skill_failures = 0
    for example in examples:
        assessed = skill_ratings.get(example["example"], [])
        score = mean(item["score"] for item in assessed) if assessed else None
        label = "通过" if score is not None and score >= 4 else "待修订" if score is not None else "待评审"
        skill_failures += label != "通过"
        lines.append(f"- `{example['example']}` · {label} · {fmt(score, 2)}")
    ledger = [row for row in writing if row["scenario"] == "03" and row["method"] == "v3"]
    ledger_unusable = sum(not row.get("writing_usable") for row in ledger)
    gate_passed = routes_ready and length_calibrated == len(profile_by_id) and not skill_failures and not ledger_unusable
    lines.extend(["", "## Phase 0 验收", "", f"结果 {'通过' if gate_passed else '待完成'}。"
                  f" 场景 03 V3 正文技术异常 {ledger_unusable} 份。待校准配置 {len(profile_by_id) - length_calibrated} 组。",
                  "80% 通过率和 4 分为本轮建议的评分阈值，可依据真实盲评结果在 ADR 中调整。"])
    (directory / "private").mkdir(exist_ok=True)
    (directory / "private" / "calibration.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    gate = {"passed": gate_passed, "pov_leaks": len(pov_fails), "missing_ratings": len(unreviewed),
            "failed_calls": len(failed), "ledger_unusable": ledger_unusable, "skill_examples_pending": skill_failures,
            "reviewed_samples": len(ratings), "reviewed_examples": len(skill_ratings)}
    (directory / "private" / "exit-gate.json").write_text(json.dumps(gate, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"Phase 0 验收 {'通过' if gate_passed else '待完成'}。校准报告见 private/calibration.md。")
    return gate_passed


def fmt(value, digits=8):
    return "未知" if value is None else f"{value:.{digits}f}"
