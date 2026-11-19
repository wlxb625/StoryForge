import contextlib
import copy
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import io
import json
import os
from pathlib import Path
import sys
import tempfile
import threading
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from benchmark_core import (
    ROOT, blocked_terms, corpus, estimate_cost, find_leaks, grade_role,
    parse_usage, read_json, request_model, writing_prompt,
)
from reports import RATING_FIELDS, SKILL_FIELDS, calibrate, csv_file, load_run, skill_examples
from run_benchmark import connect, evaluate, initialize, plan, run, run_one


class BenchmarkTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.scenarios, cls.skills, cls.cases = corpus()
        cls.config = read_json(ROOT / "profiles.example.json")

    def test_ten_scenarios_and_full_matrix(self):
        manifest, jobs = plan(self.config)
        self.assertEqual(len(manifest["scenarios"]), 10)
        self.assertEqual(sum(len(scene["beats"]) for scene in manifest["scenarios"]), 13)
        self.assertEqual(len(jobs), 172)
        self.assertEqual(manifest["input_sha256"], plan(self.config)[0]["input_sha256"])
        self.assertEqual(len({j["score_id"] for j in jobs if j["role"] == "主笔"}), 156)

    def test_ledger_pov_physical_isolation(self):
        scene = self.scenarios[2]
        beat = scene["beats"][0]
        for band in self.config["bands"]:
            traditional, _ = writing_prompt(scene, beat, self.skills, "baseline", band)
            isolated, active = writing_prompt(scene, beat, self.skills, "v3", band)
            self.assertIn("七号锅炉夹层", traditional)
            self.assertEqual(find_leaks(isolated, blocked_terms(scene, "林夏")), [])
            self.assertIn("封皮上的旧折角", isolated)
            self.assertEqual(len(active), 3)
        contaminated = {**beat, "context": "有人在等七 号、锅炉。"}
        with self.assertRaisesRegex(ValueError, "含未授权事实"):
            writing_prompt(scene, contaminated, self.skills, "v3", band)

    def test_pov_switch_and_misbelief_do_not_share_trusted_facts(self):
        scene = self.scenarios[7]
        first, _ = writing_prompt(scene, scene["beats"][0], self.skills, "v3", (700, 1100))
        second, _ = writing_prompt(scene, scene["beats"][1], self.skills, "v3", (700, 1100))
        self.assertIn("港务催验", first)
        self.assertNotIn("港务催验", second)
        self.assertIn("只怀疑船尾有人示意减速", second)
        scene = self.scenarios[3]
        prompt, _ = writing_prompt(scene, scene["beats"][0], self.skills, "v3", (700, 1100))
        self.assertIn("误认成自己哥哥", prompt)
        self.assertNotIn("松木礼堂", prompt)

    def test_reported_zero_and_unknown_are_distinct(self):
        usage = parse_usage({"usage": {"prompt_tokens": 50, "completion_tokens": 3}})
        self.assertEqual(usage, {"prompt_tokens": 50, "cached_tokens": None, "completion_tokens": 3})
        prices = {"input": 2, "cached_input": 0.2, "output": 5}
        self.assertIsNone(estimate_cost(usage, prices))
        self.assertEqual(estimate_cost({**usage, "cached_tokens": 0}, prices), 0.000115)
        self.assertEqual(estimate_cost({**usage, "cached_tokens": 20}, prices), 0.000079)
        self.assertIsNone(estimate_cost({**usage, "cached_tokens": 51}, prices))
        self.assertEqual(parse_usage({"usage": {"prompt_tokens_details": {"cached_tokens": 0}}})["cached_tokens"], 0)

    def test_role_checks_require_real_evidence(self):
        case = self.cases["提取"][0]
        self.assertTrue(grade_role(case, '[{"type":"prop_transfer","quote":"方青把黄铜钥匙交给林夏。"}]'))
        self.assertFalse(grade_role(case, '[{"type":"prop_transfer","quote":"林夏随手把窗关紧。"}]'))
        self.assertFalse(grade_role(case, '[{"type":"prop_transfer","quote":"方青把钥匙交给林夏。"}]'))
        self.assertTrue(grade_role(self.cases["审查"][2], "[]"))
        self.assertFalse(grade_role(self.cases["审查"][0], "[]"))

    def test_credential_missing_fails_before_run_creation(self):
        manifest, jobs = plan(self.config)
        with tempfile.TemporaryDirectory() as working, patch.dict(os.environ, {"STORYFORGE_A_KEY": "", "STORYFORGE_B_KEY": ""}):
            directory = Path(working) / "run"
            with self.assertRaisesRegex(ValueError, "未设置环境变量"):
                run(directory, manifest, jobs, 2, 5)
            self.assertFalse(directory.exists())

    def test_real_http_contract_with_loopback_fixture(self):
        seen = []

        class FixtureServer(BaseHTTPRequestHandler):
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                seen.append((self.path, self.headers.get("Authorization"), body))
                if body["model"] == "fixture-disconnect":
                    self.close_connection = True
                    return
                if body["model"] == "fixture-redirect":
                    self.send_response(307)
                    self.send_header("Location", "/second")
                    self.end_headers()
                    return
                output = {
                    "model": "fixture-reported-model",
                    "choices": [{"message": {"content": "林夏放下了账册。fixture-reported-model fixture-key"}, "finish_reason": "stop"}],
                    "usage": {"prompt_tokens": 25, "completion_tokens": 15, "prompt_tokens_details": {"cached_tokens": 0}},
                }
                data = json.dumps(output, ensure_ascii=False).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(data)))
                self.end_headers()
                self.wfile.write(data)

            def log_message(self, *args):
                pass

        with ThreadingHTTPServer(("127.0.0.1", 0), FixtureServer) as server:
            thread = threading.Thread(target=server.serve_forever)
            thread.start()
            try:
                config = copy.deepcopy(self.config)
                for i, profile in enumerate(config["profiles"]):
                    profile.update(base_url=f"http://127.0.0.1:{server.server_port}/v1",
                                   model=("fixture-complete", "fixture-disconnect")[i],
                                   api_key_env="STORYFORGE_FIXTURE_KEY" if i == 0 else None)
                manifest, jobs = plan(config)
                batch = [next(job for job in jobs if job["role"] == "主笔" and job["profile_id"] == profile["id"]
                              and job["scenario"] == "03" and job["method"] == "v3") for profile in config["profiles"]]
                with tempfile.TemporaryDirectory() as working, patch.dict(os.environ, {"STORYFORGE_FIXTURE_KEY": "fixture-key"}):
                    directory = Path(working) / "run"
                    with contextlib.redirect_stdout(io.StringIO()):
                        self.assertFalse(run(directory, manifest, batch, 2, 5))
                    _, rows = load_run(directory)
                    self.assertEqual([row["state"] for row in rows], ["completed", "outcome_unknown"])
                    self.assertEqual(rows[0]["cached_tokens"], 0)
                    self.assertNotIn("fixture-key", rows[0]["content"])
                    self.assertNotIn("fixture-key", (directory / "ledger.sqlite3").read_bytes().decode(errors="ignore"))
                    self.assertNotIn("fixture-reported-model", (directory / "blind-review.md").read_text(encoding="utf-8"))
                    self.assertEqual(len(seen), 2)
                    with patch("run_benchmark.request_model") as duplicate:
                        run_one(directory, 1, batch[0], config["profiles"][0], "fixture-key", 5)
                        duplicate.assert_not_called()
                    self.assertEqual(seen[0][0], "/v1/chat/completions")
                    for _, authorization, payload in seen:
                        self.assertFalse(payload["stream"])
                        self.assertNotIn("七号锅炉", json.dumps(payload, ensure_ascii=False))
                        if payload["model"] == "fixture-complete":
                            self.assertEqual(authorization, "Bearer fixture-key")
                    profile = {**config["profiles"][0], "model": "fixture-redirect"}
                    self.assertEqual(request_model(profile, "fixture-key", "body", 5)["error"], "HTTP_307")
                    self.assertEqual(len(seen), 3)
            finally:
                server.shutdown()
                thread.join(timeout=5)

    def test_calibration_requires_blind_ratings_and_no_ledger_leaks(self):
        manifest, jobs = plan(self.config)
        with tempfile.TemporaryDirectory() as working:
            directory = Path(working) / "run"
            initialize(directory, manifest, jobs)
            writing = []
            records = []
            for call_id, job in enumerate(jobs, 1):
                if job["role"] == "主笔":
                    output = "她把册子放在柜台上。"
                    writing.append({"sample": job["score_id"], **dict.fromkeys(RATING_FIELDS[1:8], 4), "pov_leak": "no"})
                else:
                    case = job["case"]
                    output = json.dumps([{"type": item, "quote": case["passage"]} for item in case["expected"]], ensure_ascii=False)
                result = {"state": "completed", "content": output, "finish_reason": "stop", "prompt_tokens": 100,
                          "cached_tokens": None, "completion_tokens": 20, "cost_usd": 0.001, "latency_ms": 100}
                evaluate(job, result)
                records.append((json.dumps(result, ensure_ascii=False), call_id))
            with connect(directory) as db:
                db.executemany("UPDATE calls SET state='completed', result_json=? WHERE call_id=?", records)
            rating = directory / "rating.csv"
            skills = directory / "skills.csv"
            csv_file(rating, [], RATING_FIELDS)
            csv_file(skills, [], SKILL_FIELDS)
            with contextlib.redirect_stdout(io.StringIO()):
                self.assertFalse(calibrate(directory, [rating], [skills]))
                csv_file(rating, writing, RATING_FIELDS)
                csv_file(skills, ({**row, "score": 4, "evidence": "单元测试记录"} for row in skill_examples(self.skills)), SKILL_FIELDS)
                self.assertTrue(calibrate(directory, [rating], [skills]))
                ledger_id = next(job["score_id"] for job in jobs if job.get("scenario") == "03" and job.get("method") == "v3")
                for row in writing:
                    if row["sample"] == ledger_id:
                        row.update(pov_leak="yes", evidence="单元测试泄密判定")
                csv_file(rating, writing, RATING_FIELDS)
                self.assertFalse(calibrate(directory, [rating], [skills]))
            self.assertEqual(read_json(directory / "private" / "exit-gate.json")["pov_leaks"], 1)


if __name__ == "__main__":
    unittest.main()
