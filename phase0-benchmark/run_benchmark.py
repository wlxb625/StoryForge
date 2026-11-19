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
from reports import calibrate, report


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


def initialize(directory, manifest, jobs):
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
    with connect(directory) as db:
        claimed = db.execute(
            "UPDATE calls SET state='running', started_at=? WHERE call_id=? AND state='queued'",
            (utc_now(), call_id),
        ).rowcount
    if not claimed:
        return
    result = request_model(profile, secret, job["prompt"], timeout)
    evaluate(job, result)
    with connect(directory) as db:
        db.execute("UPDATE calls SET state=?, finished_at=?, result_json=? WHERE call_id=?",
                   (result["state"], utc_now(), json.dumps(result, ensure_ascii=False), call_id))
    return result["state"]


def run(directory, manifest, jobs, workers, timeout):
    profiles = {profile["id"]: profile for profile in manifest["profiles"]}
    credentials = profile_credentials(profiles.values())
    manifest = {**manifest, "workers": workers, "timeout_seconds": timeout}
    initialize(directory, manifest, jobs)
    print(f"开始 {len(jobs)} 次调用。结果目录 {directory}", flush=True)
    failures = 0
    try:
        with ThreadPoolExecutor(max_workers=workers) as executor:
            futures = [
                executor.submit(run_one, directory, i, job, profiles[job["profile_id"]],
                                credentials[job["profile_id"]], timeout)
                for i, job in enumerate(jobs, 1)
            ]
            try:
                for completed, future in enumerate(as_completed(futures), 1):
                    state = future.result()
                    failures += state != "completed"
                    print(f"{completed}/{len(jobs)}  {state}", flush=True)
            except BaseException:
                for future in futures:
                    future.cancel()
                raise
    finally:
        with connect(directory) as db:
            db.execute("UPDATE calls SET state='outcome_unknown', finished_at=? WHERE state='running'", (utc_now(),))
        report(directory)
    return failures == 0


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
    for command, help_text in (("report", "从留痕重新生成盲评报告与账单"), ("calibrate", "读入人工评分并汇总三项校准")):
        sub = commands.add_parser(command, help=help_text)
        sub.add_argument("--run", required=True, type=Path)
    rating = commands.choices["calibrate"]
    rating.add_argument("--ratings", required=True, type=Path, nargs="+")
    rating.add_argument("--skills", required=True, type=Path, nargs="+")
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
                return 0 if run(directory.resolve(), manifest, jobs, args.workers, args.timeout) else 1
        elif args.command == "report":
            report(args.run.resolve())
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
