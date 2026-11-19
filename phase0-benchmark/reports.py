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

from benchmark_core import DIMENSIONS, ROOT, grade_role, price_usage, read_json, require, validate_config


TOKEN_FIELDS = ("prompt_tokens", "cached_tokens", "cache_write_tokens", "completion_tokens")
USAGE_FIELDS = (*TOKEN_FIELDS, "cost_usd", "cost_lower_usd", "cost_upper_usd", "latency_ms")
RATING_FIELDS = ("sample", *DIMENSIONS, "pov_leak", "evidence")
SKILL_FIELDS = ("example", "score", "evidence")


def load_run(directory):
    require((directory / "ledger.sqlite3").is_file(), "运行目录缺少 ledger.sqlite3")
    with closing(sqlite3.connect(directory / "ledger.sqlite3")) as db:
        run_start = db.execute("SELECT frozen_json FROM run_info").fetchone()
        require(run_start, "运行目录缺少冻结输入")
        manifest = json.loads(run_start[0])
        items = []
        for call_id, state, source, result in db.execute(
            "SELECT call_id, state, input_json, result_json FROM calls ORDER BY call_id"
        ):
            items.append({"call_id": call_id, **json.loads(source), **(json.loads(result) if result else {}), "state": state})
    prices_by_id = {p["id"]: p.get("billing_usd_per_million") for p in manifest["profiles"]}
    pricing = directory / "private" / "pricing.json"
    if pricing.is_file():
        valuation = read_json(pricing)
        require(valuation["input_sha256"] == manifest["input_sha256"], "计费单价对应其他跑测批次")
        prices_by_id.update(valuation["prices"])
    current_cases = {(role, case["id"]): case for role, cases in read_json(ROOT / "role_cases.json").items() for case in cases}
    for item in items:
        prices = prices_by_id.get(item["profile_id"])
        if prices is not None:
            item.update(price_usage({key: item.get(key) for key in TOKEN_FIELDS}, prices))
        case = item.get("case")
        updated = current_cases.get((item["role"], item.get("case_id")))
        if (item["state"] == "completed" and case and updated and "optional_events" not in case
                and {key: value for key, value in updated.items() if key != "optional_events"} == case
                and updated.get("optional_events") and item.get("finish_reason") in {None, "stop"}):
            item["original_role_pass"] = item.get("role_pass")
            item["role_pass"] = grade_role(updated, item["content"])
    return manifest, items


def reprice(directory, config):
    updated, _, _ = validate_config(config)
    manifest, _ = load_run(directory)
    routes = {profile["id"]: profile for profile in manifest["profiles"]}
    require({profile["id"] for profile in updated} == set(routes), "计费单价须对应同一组路由 ID")
    prices = {}
    for profile in updated:
        old = routes[profile["id"]]
        require(profile["model"] == old["model"] and profile["base_url"].rstrip("/") == old["base_url"].rstrip("/"),
                f"{profile['id']} 的模型或接口与冻结记录不一致")
        prices[profile["id"]] = profile.get("billing_usd_per_million")
    file = directory / "private" / "pricing.json"
    file.parent.mkdir(exist_ok=True)
    file.write_text(json.dumps({"input_sha256": manifest["input_sha256"], "prices": prices}, indent=2), encoding="utf-8")
    report(directory)


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
    mode = "w"
    if new_only and path.exists():
        with path.open(encoding="utf-8-sig", newline="") as existing:
            reader = csv.DictReader(existing)
            require(tuple(reader.fieldnames or ()) == fields, f"{path.name} 列名与模板不符")
            saved = {row[fields[0]] for row in reader}
        rows = [row for row in rows if row[fields[0]] not in saved]
        if not rows:
            return
        mode = "a"
    with path.open(mode, encoding="utf-8-sig", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=fields, extrasaction="ignore")
        if mode == "w":
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
    csv_file(private / "usage.csv", rows, ("call_id", "profile_id", "client", "transport", "request_options", "uses_system_proxy",
                                           "role", "scenario", "beat", "method", "sample",
                                           "band", "case_id", "score_id", "state", "error", "finish_reason", *USAGE_FIELDS,
                                           "cjk_chars", "writing_usable", "leak_markers", "original_role_pass", "role_pass"))
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
               "review": "pending"}
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
    values = [row.get("cost_usd") if row.get("cost_usd") is not None else row.get("cost_upper_usd") for row in rows
              if row["state"] == "completed"]
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
    failed = [row for row in rows if row["state"] in {"failed", "outcome_unknown"}]
    pending_calls = [row for row in rows if row["state"] in {"queued", "paused", "running"}]
    usage_missing = [row for row in rows if row["state"] == "completed"
                     and (row.get("prompt_tokens") is None or row.get("completion_tokens") is None)]
    unreviewed = [row for row in writing if reviewable(row) and not ratings.get(row["score_id"])]
    pov_fails = [row for row in writing if row["scenario"] == "03" and row["method"] == "v3"
                 and (row["human_leak"] or row.get("leak_markers"))]
    ledger_reviews_pending = [row for row in unreviewed if row["scenario"] == "03" and row["method"] == "v3"]
    unreviewed_skills = [example for example in examples if not skill_ratings.get(example["example"])]
    matched = defaultdict(set)
    for row in writing:
        if row.get("writing_usable") and row["mean_rating"] is not None:
            matched[(row["profile_id"], row["scenario"], row["beat"], tuple(row["band"]), row["sample"])].add(row["method"])
    covered = {(profile, scenario) for (profile, scenario, *_), methods in matched.items() if methods == {"baseline", "v3"}}
    missing_comparisons = sorted((identifier, scene["id"]) for identifier in profile_by_id
                                 for scene in manifest["scenarios"] if (identifier, scene["id"]) not in covered)
    lines = ["# StoryForge V3 Phase 0 校准", "", f"冻结输入 SHA-256 `{manifest['input_sha256']}`", "",
             "| 检查 | 未满足数 |", "| --- | ---: |",
             f"| 失败或远端结果未知 | {len(failed)} |", f"| 等待结果或尚未发送 | {len(pending_calls)} |",
             f"| 可比较场景缺口 | {len(missing_comparisons)} |", f"| 成功调用返回输入输出 token | {len(usage_missing)} |",
             f"| 费用可估算 | {sum(row.get('cost_usd') is None and row.get('cost_upper_usd') is None for row in rows)} |",
             f"| 正文待评分 | {len(unreviewed)} |", f"| 场景 03 V3 待评审 | {len(ledger_reviews_pending)} |",
             f"| 场景 03 V3 已知泄密 | {len(pov_fails)} |",
             f"| 正反例有评分 | {len(unreviewed_skills)} |", "", "## 按模型与参考篇幅", "",
             "费用采用完整用量计算的金额或缺少缓存字段时的保守上界。", "",
             "| 配置 | 方法 | 参考字数 | 通过 / 已评审 / 计划 | 通过样本实际字数范围 | 实际汉字中位数 | 盲评分均值 | 单次估算 USD |",
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
        lines.append(f"| {profile} | {method} | {low}–{high} | {passed}/{len(assessed)}/{len(group)} | {actual_range} | "
                     f"{median(counts) if counts else '未知'} | {fmt(mean(assessed) if assessed else None, 2)} | {fmt(cost)} |")
    lines.extend(["", "### 篇幅候选", ""])
    options = []
    length_calibrated = 0
    for profile in profile_by_id:
        observed_bands = {tuple(row["band"]) for row in writing
                          if row["profile_id"] == profile and row["method"] == "v3" and row.get("writing_usable")}
        eligible = [(passed / rated, score, low, high, cost)
                    for (identifier, method, low, high), (passed, _, rated, score, cost) in stats.items()
                    if identifier == profile and method == "v3" and rated and passed > 0]
        length_calibrated += len(observed_bands) == len(manifest["bands"])
        if eligible:
            rate, score, low, high, cost = max(eligible, key=lambda item: (item[0], item[1], -abs(item[3] - item[2])))
            lines.append(f"- {profile} · {low}–{high} 汉字 · 通过率 {rate:.0%} · 均分 {score:.2f}")
            if rate >= 0.8 and cost is not None:
                options.append((profile, cost))
        else:
            lines.append(f"- {profile} · 本轮暂无通过评审的篇幅区间。已取得 {len(observed_bands)} 档完整输出。")
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
    if any("original_role_pass" in row for row in rows):
        lines.append("死亡题例允许同时报告有原文证据的死亡证明交接。原始调用和首判保留在日志中，补充口径仅离线重判。")
    routes_ready = bool(options and candidates["审查"] and candidates["提取"]
                        and not (usage_missing or ledger_reviews_pending or pov_fails or pending_calls or missing_comparisons))
    if routes_ready:
        writer, writing_cost = min(options, key=lambda item: item[1])
        audit_cost, _, audit = min(candidates["审查"])
        memory_cost, _, memory = min(candidates["提取"])
        lines.append(f"成本候选组合为主笔 {writer}、审查 {audit}、提取 {memory}。按每角色各调用一次估算上界 "
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
    ledger_covered = {row["profile_id"] for row in ledger if row.get("writing_usable") and row["mean_rating"] is not None}
    ledger_without_sample = len(profile_by_id.keys() - ledger_covered)
    gate_passed = routes_ready and length_calibrated == len(profile_by_id) and not skill_failures and not ledger_without_sample
    lines.extend(["", "## Phase 0 验收", "", f"结果 {'通过' if gate_passed else '待完成'}。"
                  f" 场景 03 V3 缺少已评审完整样本的配置 {ledger_without_sample} 组。待校准配置 {len(profile_by_id) - length_calibrated} 组。",
                  "80% 通过率和 4 分为本轮建议的评分阈值，可依据真实盲评结果在 ADR 中调整。"])
    if missing_comparisons:
        lines.extend(["", "缺少同条件已评审对照的场景 " + "、".join(f"{identifier}/{scene}" for identifier, scene in missing_comparisons)])
    (directory / "private").mkdir(exist_ok=True)
    (directory / "private" / "calibration.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    gate = {"passed": gate_passed, "pov_leaks": len(pov_fails), "missing_ratings": len(unreviewed),
            "ledger_reviews_pending": len(ledger_reviews_pending),
            "failed_calls": len(failed), "pending_calls": len(pending_calls), "missing_comparisons": missing_comparisons,
            "ledger_without_sample": ledger_without_sample, "skill_examples_pending": skill_failures,
            "reviewed_samples": len(ratings), "reviewed_examples": len(skill_ratings)}
    (directory / "private" / "exit-gate.json").write_text(json.dumps(gate, ensure_ascii=False, indent=2), encoding="utf-8")
    print(f"Phase 0 验收 {'通过' if gate_passed else '待完成'}。校准报告见 private/calibration.md。")
    return gate_passed


def fmt(value, digits=8):
    return "未知" if value is None else f"{value:.{digits}f}"
