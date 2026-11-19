"""Run the Phase 0 matrix and checkpoint calls in a local SQLite ledger."""

import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
from contextlib import closing, contextmanager
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import random
import sqlite3
import sys
from uuid import uuid4

import yaml

from benchmark_core import (
    ROOT, TERMINAL_PUNCTUATION, blocked_terms, corpus, count_han, find_leaks,
    grade_role, profile_credentials, read_json, request_model, require, role_prompt,
    validate_config, writing_prompt,
)
from reports import calibrate, report, reprice


def utc_now():
    return datetime.now(timezone.utc).isoformat(timespec="milliseconds")


def plan(config):
    profiles, bands, repeats = validate_config(config)
    scenarios, skills, cases = corpus()
    jobs = []
    for profile in profiles:
        for sample in range(1, repeats + 1):
            for scene in scenarios:
                for index, beat in enumerate(scene["beats"], 1):
                    for band in bands:
                        for method in ("baseline", "v3"):
                            prompt, active = writing_prompt(scene, beat, skills, method, band)
                            jobs.append({
                                "profile_id": profile["id"], "role": "主笔", "score_id": uuid4().hex[:12],
                                "scenario": scene["id"], "beat": index, "method": method, "sample": sample,
                                "band": band, "pov": beat["pov"], "prompt": prompt, "skills": active,
                                "blocked": blocked_terms(scene, beat["pov"]),
                            })
            for role, role_cases in cases.items():
                for case in role_cases:
                    jobs.append({
                        "profile_id": profile["id"], "role": role, "case_id": case["id"], "sample": sample,
                        "prompt": role_prompt(role, case), "case": case,
                    })
    seed = config.get("shuffle_seed", 1319)
    require(type(seed) is int, "shuffle_seed 必须是整数")
    random.Random(seed).shuffle(jobs)
    frozen_inputs = {"scenarios": scenarios, "skills": skills, "role_cases": cases,
                     "profiles": profiles, "bands": bands, "repetitions": repeats, "shuffle_seed": seed}
    frozen_requests = [{k: v for k, v in job.items() if k != "score_id"} for job in jobs]
    fingerprint = json.dumps([frozen_inputs, frozen_requests], ensure_ascii=False, sort_keys=True).encode()
    digest = hashlib.sha256(fingerprint).hexdigest()
    return {"created_at": utc_now(), "input_sha256": digest, **frozen_inputs}, jobs


@contextmanager
def connect(directory):
    with closing(sqlite3.connect(directory / "ledger.sqlite3", timeout=30)) as db, db:
        yield db


def reuse_calls(source, manifest, jobs):
    require((source / "ledger.sqlite3").is_file(), "原跑测目录缺少 ledger.sqlite3")
    with connect(source) as db:
        old_manifest = json.loads(db.execute("SELECT frozen_json FROM run_info").fetchone()[0])
        original_calls = db.execute(
            "SELECT call_id, input_json, state, started_at, finished_at, result_json FROM calls"
        ).fetchall()
    original_profiles = {p["id"]: p for p in old_manifest["profiles"]}
    compare_fields = ("base_url", "model", "api_key_env", "options")
    reuse_profiles = {
        p["id"] for p in manifest["profiles"] if p["id"] in original_profiles
        and all(p.get(field) == original_profiles[p["id"]].get(field) for field in compare_fields)
    }
    require(reuse_profiles, "新旧跑测没有相同路由")
    fields = ("profile_id", "role", "case_id", "scenario", "beat", "method", "band", "sample")
    def job_key(job):
        return json.dumps([job.get(key) for key in fields], ensure_ascii=False)

    new_jobs = {job_key(job): (index, job) for index, job in enumerate(jobs, 1)}
    carried = []
    for old_id, source_json, state, start, finish, result in original_calls:
        previous = json.loads(source_json)
        if previous["profile_id"] not in reuse_profiles:
            continue
        require(state not in {"running", "queued"}, "请先暂停原批次的未发送请求并等待在途请求结束")
        if state not in {"completed", "failed", "outcome_unknown"}:
            continue
        replacement = new_jobs.get(job_key(previous))
        if replacement is None:
            continue
        index, planned = replacement
        require(all(planned.get(field) == previous.get(field) for field in ("prompt", "skills", "blocked", "case")),
                "原批次的提示词或评审条件已变化")
        previous["source_call"] = {"input_sha256": old_manifest["input_sha256"], "call_id": old_id}
        carried.append((json.dumps(previous, ensure_ascii=False), state, start, finish, result, index))
    return carried


def initialize(directory, manifest, jobs, reuse_from=None):
    carried = reuse_calls(reuse_from, manifest, jobs) if reuse_from else []
    directory.mkdir(parents=True, exist_ok=False)
    with connect(directory) as db:
        db.execute("PRAGMA journal_mode=WAL")
        db.executescript("""
            CREATE TABLE run_info (frozen_json TEXT NOT NULL);
            CREATE TABLE calls (
                call_id INTEGER PRIMARY KEY,
                input_json TEXT NOT NULL,
                state TEXT NOT NULL DEFAULT 'queued',
                started_at TEXT,
                finished_at TEXT,
                result_json TEXT
            );
        """)
        db.execute("INSERT INTO run_info VALUES (?)", (json.dumps(manifest, ensure_ascii=False),))
        db.executemany("INSERT INTO calls (input_json) VALUES (?)", ((json.dumps(job, ensure_ascii=False),) for job in jobs))
        db.executemany("UPDATE calls SET input_json=?, state=?, started_at=?, finished_at=?, result_json=? WHERE call_id=?", carried)
    return len(carried)


def evaluate(job, result):
    output = result["content"]
    if job["role"] != "主笔":
        result["role_pass"] = (result["state"] == "completed" and result["finish_reason"] in {None, "stop"}
                               and grade_role(job["case"], output))
        return
    result["cjk_chars"] = count_han(output)
    result["leak_markers"] = find_leaks(output, job["blocked"])
    result["writing_usable"] = (
        result["state"] == "completed" and result["finish_reason"] in {None, "stop"}
        and result["cjk_chars"] > 0 and output.rstrip()[-1] in TERMINAL_PUNCTUATION
        and not output.lstrip().startswith(("```", "<think>"))
    )


def run_one(directory, call_id, job, profile, secret, timeout):
    job = {**job, "transport": "sse" if profile.get("streaming") else "json", "request_options": profile.get("options", {})}
    job["uses_system_proxy"] = profile.get("use_system_proxy", True)
    job["client"] = profile.get("http_client", "urllib")
    with connect(directory) as db:
        claimed = db.execute(
            "UPDATE calls SET state='running', started_at=?, input_json=? WHERE call_id=? AND state='queued'",
            (utc_now(), json.dumps(job, ensure_ascii=False), call_id),
        ).rowcount
    if not claimed:
        return
    result = request_model(profile, secret, job["prompt"], timeout)
    evaluate(job, result)
    with connect(directory) as db:
        db.execute("UPDATE calls SET state=?, finished_at=?, result_json=? WHERE call_id=?",
                   (result["state"], utc_now(), json.dumps(result, ensure_ascii=False), call_id))
    return result["state"], result.get("error")


def run(directory, manifest, jobs, workers, timeout, reuse_from=None):
    profile_credentials(manifest["profiles"])
    manifest = {**manifest, "workers": workers, "timeout_seconds": timeout}
    carried = initialize(directory, manifest, jobs, reuse_from)
    print(f"计划 {len(jobs)} 个样本，沿用 {carried} 条已发送记录。结果目录 {directory}", flush=True)
    return resume(directory, timeout, workers=workers)


def resume(directory, timeout, profile_id=None, streaming=None, max_tokens=None, limit=None, direct=False, workers=16, curl=False):
    require((directory / "ledger.sqlite3").is_file(), "运行目录缺少 ledger.sqlite3")
    route_filter = " AND json_extract(input_json, '$.profile_id')=?" if profile_id else ""
    route_args = (profile_id,) if profile_id else ()
    with connect(directory) as db:
        manifest = json.loads(db.execute("SELECT frozen_json FROM run_info").fetchone()[0])
        require(not profile_id or any(p["id"] == profile_id for p in manifest["profiles"]), "本轮不存在指定路由")
        require(not db.execute("SELECT 1 FROM calls WHERE state='running'" + route_filter + " LIMIT 1", route_args).fetchone(),
                "选定路由仍有在途调用。等调用结束后再继续。")
        pending = [
            (call_id, json.loads(job)) for call_id, job in db.execute(
                "SELECT call_id, input_json FROM calls WHERE state IN ('queued','paused')" + route_filter + " ORDER BY call_id LIMIT ?",
                (*route_args, limit or -1),
            )
        ]
    if not pending:
        print("本轮没有未发送的样本。")
        report(directory)
        with connect(directory) as db:
            return not db.execute("SELECT 1 FROM calls WHERE state!='completed' LIMIT 1").fetchone()

    profiles = {p["id"]: {**p, "streaming": streaming} if streaming is not None else p for p in manifest["profiles"]}
    if direct:
        profiles[profile_id] = {**profiles[profile_id], "use_system_proxy": False}
    if curl:
        profiles[profile_id] = {**profiles[profile_id], "http_client": "curl"}
    if max_tokens is not None:
        profile = profiles[profile_id]
        options = {key: value for key, value in profile.get("options", {}).items() if key != "max_completion_tokens"}
        profiles[profile_id] = {**profile, "options": {**options, "max_tokens": max_tokens}}
    credentials = profile_credentials(p for p in profiles.values() if any(j["profile_id"] == p["id"] for _, j in pending))
    with connect(directory) as db:
        db.execute("UPDATE calls SET state='queued' WHERE state='paused'" + route_filter, route_args)
    print(f"继续 {len(pending)} 个未发送样本，每条路由同时运行一个请求。", flush=True)

    def run_route(profile):
        profile_id = profile["id"]
        consecutive_errors = 0
        for call_id, job in pending:
            if job["profile_id"] != profile_id:
                continue
            result = run_one(directory, call_id, job, profile, credentials[profile_id], timeout)
            if result is None:
                continue
            state, error = result
            print(f"{profile_id} #{call_id} {state} {error or ''}".rstrip(), flush=True)
            consecutive_errors = 0 if state == "completed" else consecutive_errors + 1
            if error in {"HTTP_401", "HTTP_403"} or consecutive_errors >= 3:
                with connect(directory) as db:
                    db.execute(
                        "UPDATE calls SET state='paused' WHERE state='queued' AND json_extract(input_json, '$.profile_id')=?",
                        (profile_id,),
                    )
                print(f"{profile_id} 已暂停尚未发送的请求，最近错误 {error or state}。", flush=True)
                break

    try:
        with ThreadPoolExecutor(max_workers=min(workers, len(credentials))) as executor:
            futures = [executor.submit(run_route, profiles[identifier]) for identifier in credentials]
            try:
                for future in as_completed(futures):
                    future.result()
            except BaseException:
                with connect(directory) as db:
                    db.execute("UPDATE calls SET state='paused' WHERE state='queued'" + route_filter, route_args)
                raise
    finally:
        report(directory)
    with connect(directory) as db:
        return not db.execute("SELECT 1 FROM calls WHERE state!='completed' LIMIT 1").fetchone()


def main():
    for pipe in (sys.stdout, sys.stderr):
        pipe.reconfigure(errors="backslashreplace")
    parser = argparse.ArgumentParser(description="StoryForge V3 Phase 0 多模型创作盲评")
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("validate", help="离线校验十个场景、Skill 和 POV 隔离")
    for command, help_text in (("plan", "列出调用规模，不发请求"), ("run", "运行真实模型跑测")):
        sub = commands.add_parser(command, help=help_text)
        sub.add_argument("--config", required=True, type=Path)
    execute = commands.choices["run"]
    execute.add_argument("--out", type=Path, help="新的输出目录，默认在 runs 下新建")
    execute.add_argument("--workers", type=int, default=2)
    execute.add_argument("--timeout", type=int, default=180)
    execute.add_argument("--reuse-from", type=Path, help="替换路由时沿用旧批次中未变更路由的已发送记录")
    resume_parser = commands.add_parser("resume", help="逐路由继续本轮尚未发送的请求")
    resume_parser.add_argument("--run", required=True, type=Path)
    resume_parser.add_argument("--timeout", type=int, default=180)
    resume_parser.add_argument("--profile", help="只继续指定路由")
    transports = resume_parser.add_mutually_exclusive_group()
    transports.add_argument("--stream", dest="streaming", action="store_const", const=True, help="对指定路由改用流式传输")
    transports.add_argument("--json", dest="streaming", action="store_const", const=False, help="对指定路由改用普通 JSON")
    resume_parser.add_argument("--max-tokens", type=int, help="对尚未发送的样本设置输出 token 上限")
    resume_parser.add_argument("--limit", type=int, help="只继续指定数量的尚未发送样本")
    resume_parser.add_argument("--direct", action="store_true", help="指定路由直接连接，不使用系统代理")
    resume_parser.add_argument("--curl", action="store_true", help="指定路由使用系统 curl")
    for command, help_text in (("report", "从留痕重新生成盲评报告与账单"), ("calibrate", "读入人工评分并汇总三项校准"),
                               ("reprice", "按本次路由的确认价格重算账单，无需重新调用模型")):
        sub = commands.add_parser(command, help=help_text)
        sub.add_argument("--run", required=True, type=Path)
    rating = commands.choices["calibrate"]
    rating.add_argument("--ratings", required=True, type=Path, nargs="+")
    rating.add_argument("--skills", required=True, type=Path, nargs="+")
    commands.choices["reprice"].add_argument("--config", required=True, type=Path)
    args = parser.parse_args()
    try:
        if args.command == "validate":
            scenarios, skills, cases = corpus()
            print(f"通过离线校验：{len(scenarios)} 场景、{sum(len(s['beats']) for s in scenarios)} 节拍、"
                  f"{len(skills)} 项 Skill、{sum(map(len, cases.values()))} 审查及提取案例。")
        elif args.command in {"plan", "run"}:
            manifest, jobs = plan(read_json(args.config))
            print(f"{len(manifest['profiles'])} 组模型配置，{len(jobs)} 次调用，含两种写作方法、"
                  "三个参考篇幅以及独立审查和提取。")
            if args.command == "run":
                require(1 <= args.workers <= 16 and 5 <= args.timeout <= 1800, "并发 1 至 16，超时 5 至 1800 秒")
                directory = args.out or ROOT / "runs" / (utc_now().replace("+00:00", "Z").replace(":", "") + "-" + uuid4().hex[:6])
                return 0 if run(directory.resolve(), manifest, jobs, args.workers, args.timeout, args.reuse_from) else 1
        elif args.command == "report":
            report(args.run.resolve())
        elif args.command == "resume":
            require(5 <= args.timeout <= 1800, "超时须为 5 至 1800 秒")
            require(not (args.streaming is not None or args.max_tokens is not None or args.direct or args.curl) or args.profile,
                    "传输和参数覆盖须指定 --profile")
            require(args.max_tokens is None or args.max_tokens > 0, "--max-tokens 须为正整数")
            require(args.limit is None or args.limit > 0, "--limit 须为正整数")
            return 0 if resume(args.run.resolve(), args.timeout, args.profile, args.streaming, args.max_tokens,
                               args.limit, args.direct, curl=args.curl) else 1
        elif args.command == "reprice":
            reprice(args.run.resolve(), read_json(args.config))
        else:
            return 0 if calibrate(args.run.resolve(), args.ratings, args.skills) else 1
    except (OSError, ValueError, KeyError, TypeError, yaml.YAMLError, sqlite3.Error) as error:
        print(f"运行失败（{type(error).__name__}） {error}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("已停止，开始过的调用已留痕。", file=sys.stderr)
        return 130
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
