"""Frozen benchmark inputs, POV compilation and OpenAI-compatible requests."""

import itertools
import io
import json
import math
import os
import re
import shutil
import subprocess
import time
import unicodedata
from http.client import HTTPException
from pathlib import Path
from urllib.error import HTTPError, URLError
from urllib.parse import urlsplit
from urllib.request import HTTPRedirectHandler, ProxyHandler, Request, build_opener

import yaml

ROOT = Path(__file__).resolve().parent
BASE_STYLE = "skill.style.grounded"
DIMENSIONS = ("plot", "character", "pov", "continuity", "emotion", "dialogue", "style")
MODELS_OPTIONS = {"temperature", "top_p", "max_tokens", "max_completion_tokens", "reasoning_effort"}
EXAMPLE_HEADINGS = ("## 核心技法", "## 正例", "## 反例")
SKILL_FIELDS = {"id", "name", "version", "slot", "priority", "triggers", "anti_triggers", "avoid_phrases"}
TERMINAL_PUNCTUATION = frozenset("。！？…!?.”’」』）》）\"'")


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8-sig"))


def require(condition, message):
    if not condition:
        raise ValueError(message)


def load_skills():
    skills = {}
    for path in sorted((ROOT / "skills").glob("*.md")):
        start, metadata, body = path.read_text(encoding="utf-8").split("---\n", maxsplit=2)
        spec = yaml.safe_load(metadata)
        require(not start and isinstance(spec, dict), f"{path.name} 缺少 YAML Frontmatter")
        require(set(spec) <= SKILL_FIELDS, f"{path.name} 含未定义的 Skill 字段")
        for field in ("id", "name", "version"):
            require(isinstance(spec.get(field), str) and spec[field].strip(), f"{path.name} 缺少 {field}")
        require(re.fullmatch(r"\d+\.\d+\.\d+", spec["version"]), f"{path.name} 版本格式错误")
        require(spec["slot"] in {"base_style", "technique"}, f"{path.name} 槽位错误")
        require(type(spec["priority"]) is int and 1 <= spec["priority"] <= 100, f"{path.name} 优先级错误")
        for field in ("triggers", "anti_triggers", "avoid_phrases"):
            values = spec.get(field, [])
            require(isinstance(values, list) and all(isinstance(v, str) for v in values), f"{path.name} {field} 格式错误")
            spec[field] = values
        sections = re.split(r"^## .+$", body, flags=re.MULTILINE)
        require(re.findall(r"^## .+$", body, re.MULTILINE) == list(EXAMPLE_HEADINGS), f"{path.name} 分节错误")
        _, rules, positive, negative = sections
        positives = re.split(r"\n\s*\n", positive.strip())
        negatives = re.split(r"\n\s*\n", negative.strip())
        require(rules.strip() and len(positives) == 2 and all(p.startswith("> ") for p in positives),
                f"{path.name} 需要两段正例")
        require(1 <= len(negatives) <= 2 and all(p.startswith("> ") and "错因" in p for p in negatives),
                f"{path.name} 需要一到两段带错因的反例")
        require(spec["id"] not in skills, f"{path.name} Skill ID 重复")
        skills[spec["id"]] = {**spec, "body": body.strip()}
    require(len(skills) >= 3 and BASE_STYLE in skills, "缺少 Phase 0 基线 Skill")
    return skills


def normalize(text):
    return "".join(char for char in unicodedata.normalize("NFKC", text).casefold() if char.isalnum())


def count_han(text):
    return sum(unicodedata.name(char, "").startswith(("CJK UNIFIED IDEOGRAPH", "CJK COMPATIBILITY IDEOGRAPH"))
               for char in text)


def blocked_terms(scenario, pov):
    return [term for fact in scenario.get("facts", []) if fact["views"].get(pov, {}).get("state") != "knows"
            for term in [fact["truth"], *fact["leak_markers"]]]


def find_leaks(text, terms):
    normalized = normalize(text)
    return [term for term in terms if normalize(term) in normalized]


def writing_prompt(scenario, beat, skills, method, band):
    pov = beat["pov"]
    low, high = band
    lines = [
        "为中文长篇小说写一个节拍。只输出正文。",
        f"视角人物：{pov}。采用限知视角，仅叙述此人能感知、回忆或推断的内容。",
        f"背景：{scenario['public']}",
        f"本拍起点：{beat['context']}",
        f"写作目标：{beat['goal']}",
        f"基调：{beat['tone']}",
        f"参考长度：{low}–{high} 个汉字。让这个节拍自然结束。",
    ]
    for fact in scenario.get("facts", []):
        view = fact["views"].get(pov, {})
        state = view.get("state")
        if method == "baseline":
            lines.append(f"作者掌握的事实：{fact['truth']}。视角人物状态：{state or '未知'}。遵守视角限制。")
        elif state == "knows":
            lines.append(f"此人已知：{fact['truth']}")
        if state in {"suspects", "misbelieves"}:
            label = "此人的怀疑" if state == "suspects" else "此人目前的看法，保留不确定性"
            lines.append(f"{label}：{view['text']}")
        lines.extend(f"可见线索：{clue}" for clue in fact["observable_clues"])
    active = []
    if method == "v3":
        techniques = [skills[item] for item in beat["techniques"]]
        active = [skills[BASE_STYLE], *sorted(techniques, key=lambda item: -item["priority"])]
        lines.append("约束优先级由高到低：知情边界与既定事实、本拍目标与基调、以下文笔技法。")
        for skill in active:
            lines.append(f"写法参考 {skill['name']}：\n{skill['body']}")
            if skill["avoid_phrases"]:
                lines.append("留意套话：" + "、".join(skill["avoid_phrases"]))
    prompt = "\n\n".join(lines)
    if method == "v3" and find_leaks(prompt, blocked_terms(scenario, pov)):
        raise ValueError(f"场景 {scenario['id']} 的 {pov} 生成 Prompt 含未授权事实")
    return prompt, [{"id": item["id"], "version": item["version"]} for item in active]


def corpus():
    skills = load_skills()
    scenarios = [read_json(path) for path in sorted((ROOT / "scenarios").glob("*.json"))]
    require([scenario["id"] for scenario in scenarios] == [f"{i:02}" for i in range(1, 11)], "须有编号 01 至 10 的十个场景")
    for scenario in scenarios:
        require(scenario["beats"], f"场景 {scenario['id']} 无节拍")
        for fact in scenario.get("facts", []):
            require(fact["truth"] and fact["leak_markers"] and all(normalize(term) for term in fact["leak_markers"]),
                    f"场景 {scenario['id']} 缺少保密校验词")
            for view in fact["views"].values():
                require(view["state"] in {"knows", "suspects", "misbelieves"}, "非法知情状态")
                require(view["state"] == "knows" or view.get("text"), "怀疑和误解需要独立文本")
        for beat in scenario["beats"]:
            techniques = beat["techniques"]
            require(len(techniques) == len(set(techniques)) <= 2
                    and all(skills[item]["slot"] == "technique" for item in techniques), "非法技法槽位")
            writing_prompt(scenario, beat, skills, "v3", (700, 1100))
    ledger = scenarios[2]
    require(ledger["beats"][0]["pov"] == "林夏" and "调包账本" in blocked_terms(ledger, "林夏"),
            "场景 03 缺少林夏 POV 对抗用例")
    return scenarios, skills, read_json(ROOT / "role_cases.json")


def validate_config(config):
    models = config["profiles"]
    require(isinstance(models, list) and len(models) >= 2, "对比跑测至少需要两组配置")
    ids = set()
    for profile in models:
        require(set(profile) <= {"id", "model", "base_url", "api_key_env", "billing_usd_per_million",
                                 "options", "streaming", "use_system_proxy", "http_client"},
                "profile 存在不支持的字段，密钥请通过环境变量传入")
        identifier = profile["id"]
        require(isinstance(identifier, str) and re.fullmatch(r"[A-Za-z0-9_-]{1,40}", identifier) and identifier not in ids,
                "profile.id 必须唯一，且只使用 ASCII 字母、数字、下划线或横线")
        ids.add(identifier)
        require(isinstance(profile["model"], str) and profile["model"].strip(), f"{identifier} 未填模型名")
        url = urlsplit(profile["base_url"])
        loopback = url.hostname in {"localhost", "127.0.0.1", "::1"}
        require(url.hostname and (url.scheme == "https" or (url.scheme == "http" and loopback))
                and url.username is None and not url.query and not url.fragment,
                f"{identifier} 需要 HTTPS 或本机 HTTP，地址中不能包含凭据或查询参数")
        env_name = profile.get("api_key_env")
        require(env_name is None or (isinstance(env_name, str) and re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", env_name)),
                f"{identifier} 的 api_key_env 必须是环境变量名")
        options = profile.get("options", {})
        require(type(profile.get("streaming", False)) is bool, f"{identifier} 的 streaming 须为布尔值")
        require(type(profile.get("use_system_proxy", True)) is bool, f"{identifier} 的 use_system_proxy 须为布尔值")
        require(profile.get("http_client", "urllib") in {"urllib", "curl"}, f"{identifier} 的 HTTP 客户端配置无效")
        require(isinstance(options, dict) and set(options) <= MODELS_OPTIONS
                and all(type(value) in {int, float, str} for value in options.values()),
                f"{identifier} 传入了不受支持的模型参数")
        prices = profile.get("billing_usd_per_million")
        if prices is not None:
            require(isinstance(prices, dict) and {"input", "output"} <= set(prices)
                    <= {"input", "output", "cached_input", "cache_write", "max_prompt_tokens"}
                    and all(type(rate) in {float, int} and math.isfinite(rate) and rate >= 0 for rate in prices.values()),
                    f"{identifier} 价格必须为每百万 token 的非负美元数")
            ceiling = prices.get("max_prompt_tokens")
            require(ceiling is None or (type(ceiling) is int and ceiling > 0), f"{identifier} 价格档位的输入 token 上限无效")
    require(len({(p["base_url"].rstrip("/"), p["model"]) for p in models}) == len(models),
            "同端点同模型重复配置，无法形成独立对照")
    bands = config.get("bands", [[400, 600], [700, 1100], [1200, 1600]])
    require(isinstance(bands, list) and len(bands) == 3 and all(
        isinstance(band, list) and len(band) == 2 and all(type(n) is int for n in band)
        and 100 <= band[0] < band[1] <= 3000 for band in bands), "需要三个有效的汉字长度区间")
    require(all(a[1] < b[0] for a, b in itertools.pairwise(bands)), "长度区间必须按序排列且互不重叠")
    repetitions = config.get("repetitions", 1)
    require(type(repetitions) is int and 1 <= repetitions <= 10, "repetitions 须为 1 到 10")
    return models, bands, repetitions


def curl_executable():
    return shutil.which("curl.exe" if os.name == "nt" else "curl")


def profile_credentials(profiles):
    credentials = {}
    for profile in profiles:
        require(profile.get("http_client") != "curl" or curl_executable(), "未找到系统 curl，无法使用已选择的 HTTP 客户端")
        env_name = profile.get("api_key_env")
        secret = os.environ.get(env_name, "") if env_name else ""
        require(not env_name or secret.strip(), f"未设置环境变量 {env_name}")
        require("\r" not in secret and "\n" not in secret, f"环境变量 {env_name} 内容格式不正确")
        credentials[profile["id"]] = secret
    return credentials


def role_prompt(role, case):
    rule = {
        "审查": (
            "检查 POV 泄密与 Canon 冲突。只返回 JSON 数组。"
            '每条包含 type（pov_leak 或 canon_conflict）和 quote（正文中的连续原文）。无冲突时返回 []。'
        ),
        "提取": (
            "提取本段实际发生的道具交接、地点移动、死亡。忽略假设、意图和未发生事件。只返回 JSON 数组。"
            '每项包含 type（prop_transfer、location_move 或 death）和 quote（包含当事人及事件的连续原文）。无事件时返回 []。'
        ),
    }[role]
    return f"{rule}\n\n设定：{case['context']}\n正文：{case['passage']}"


def grade_role(case, output):
    try:
        answers = json.loads(re.sub(r"^\s*```(?:json)?\s*|\s*```\s*$", "", output))
    except ValueError:
        return False
    if not isinstance(answers, list) or len(answers) != len(case["expected"]):
        return False
    return all(
        isinstance(answer, dict) and answer.get("type") == expected
        and isinstance(quote := answer.get("quote"), str) and quote.strip() and quote in case["passage"]
        and all(fragment in quote for fragment in case.get("required_quotes", []))
        for answer, expected in zip(answers, case["expected"], strict=True)
    )


def parse_usage(response):
    usage = response.get("usage")
    usage = usage if isinstance(usage, dict) else {}
    details = usage.get("prompt_tokens_details")
    details = details if isinstance(details, dict) else {}

    def count(value):
        return value if type(value) is int and value >= 0 else None

    cached = count(details.get("cached_tokens"))
    if cached is None:
        cached = count(usage.get("prompt_cache_hit_tokens"))
    return {
        "prompt_tokens": count(usage.get("prompt_tokens")),
        "cached_tokens": cached,
        "cache_write_tokens": count(details.get("cache_write_tokens")),
        "completion_tokens": count(usage.get("completion_tokens")),
    }


def price_usage(usage, prices):
    prompt, cached, completion = (usage[key] for key in ("prompt_tokens", "cached_tokens", "completion_tokens"))
    unknown = {"cost_usd": None, "cost_lower_usd": None, "cost_upper_usd": None}
    if prices is None or prompt is None or completion is None:
        return unknown
    if prompt > prices.get("max_prompt_tokens", prompt):
        return unknown
    uncached = prompt
    input_rates = [prices["input"]]
    metered = completion * prices["output"]
    for count, rate in (
        (cached, prices.get("cached_input", prices["input"])),
        (usage.get("cache_write_tokens"), prices.get("cache_write", prices["input"])),
    ):
        if count is None:
            input_rates.append(rate)
        else:
            uncached -= count
            metered += count * rate
    if uncached < 0:
        return unknown
    low, high = (round((metered + uncached * rate) / 1e6, 12) for rate in (min(input_rates), max(input_rates)))
    return {"cost_usd": low if low == high else None, "cost_lower_usd": low, "cost_upper_usd": high}


def estimate_cost(usage, prices):
    return price_usage(usage, prices)["cost_usd"]


class NoRedirect(HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def read_stream(response, result, usage):
    content = []
    event_data = []
    received = 0
    finished = False
    try:
        while (line := response.readline(1024 * 1024 + 1)):
            received += len(line)
            require(len(line) <= 1024 * 1024 and received <= 8 * 1024 * 1024, "SSE 响应超过大小限制")
            if line.startswith(b"data:"):
                event_data.append(line[5:].strip())
            if line.strip() or not event_data:
                continue
            event = b"\n".join(event_data)
            event_data.clear()
            if event == b"[DONE]":
                finished = True
                break
            body = json.loads(event)
            require(isinstance(body, dict) and not body.get("error"), "SSE 事件无效")
            if isinstance(body.get("model"), str):
                result["reported_model"] = body["model"]
            if isinstance(body.get("usage"), dict):
                for key, value in parse_usage(body).items():
                    if value is not None:
                        usage[key] = value
            choices = body.get("choices") or []
            if choices:
                text = choices[0].get("delta", {}).get("content")
                if isinstance(text, str):
                    content.append(text)
                if choices[0].get("finish_reason") in ("stop", "length", "content_filter", "tool_calls", "function_call"):
                    result["finish_reason"] = choices[0]["finish_reason"]
        finished = finished or (result["finish_reason"] is not None and all(
            usage[key] is not None for key in ("prompt_tokens", "completion_tokens")))
        result["content"] = "".join(content)
        if not finished:
            result["error"] = "INCOMPLETE_STREAM"
        elif result["content"].strip():
            result["state"] = "completed"
        else:
            result.update(state="failed", error="NO_TEXT")
    finally:
        result["content"] = "".join(content)


def read_json_response(response, result, usage):
    raw = response.read(4 * 1024 * 1024 + 1)
    require(len(raw) <= 4 * 1024 * 1024, "响应超过 4 MiB")
    body = json.loads(raw)
    require(isinstance(body, dict), "响应不是 JSON 对象")
    usage.update(parse_usage(body))
    choice = (body.get("choices") or [{}])[0]
    content = choice.get("message", {}).get("content")
    if isinstance(content, list):
        content = "".join(part["text"] for part in content if isinstance(part, dict) and isinstance(part.get("text"), str))
    reason = choice.get("finish_reason")
    text = content if isinstance(content, str) else ""
    result.update(state="completed" if text.strip() else "failed", content=text,
                  reported_model=body.get("model") if isinstance(body.get("model"), str) else None,
                  finish_reason=reason if reason in ("stop", "length", "content_filter", "tool_calls", "function_call") else None)
    if result["state"] == "failed":
        result["error"] = "NO_TEXT"


def http_error(result, status):
    result.update(state="outcome_unknown" if status >= 500 or status == 408 else "failed", error=f"HTTP_{status}")


def request_with_curl(url, headers, payload, timeout, direct, result, usage):
    def encoded(value):
        return json.dumps(value, ensure_ascii=False)

    settings = [f"url = {encoded(url)}", 'request = "POST"', f"data-binary = {encoded(encoded(payload))}"]
    settings.extend(f"header = {encoded(name + ': ' + value)}" for name, value in headers.items())
    cmd = [curl_executable(), "--disable", "--config", "-", "--silent", "--max-time", str(timeout),
           "--max-filesize", str(8 * 1024 * 1024), "--write-out", "\n%{http_code}\t%{content_type}"]
    if direct:
        cmd.extend(("--noproxy", "*"))
    response = subprocess.run(cmd, input=("\n".join(settings) + "\n").encode(), capture_output=True,
                              timeout=timeout + 5, creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0)
    raw, _, metadata = response.stdout.rpartition(b"\n")
    status, _, media_type = metadata.partition(b"\t")
    if status.isdigit() and int(status) >= 300:
        http_error(result, int(status))
    elif payload["stream"] and media_type.startswith(b"text/event-stream"):
        read_stream(io.BytesIO(raw), result, usage)
        if response.returncode:
            result.update(state="outcome_unknown", error=f"CURL_{response.returncode}")
    elif response.returncode:
        result["error"] = f"CURL_{response.returncode}"
    else:
        read_json_response(io.BytesIO(raw), result, usage)


def request_model(profile, secret, prompt, timeout):
    url = profile["base_url"].rstrip("/")
    if not url.endswith("/chat/completions"):
        url += "/chat/completions"
    streaming = profile.get("streaming", False)
    payload = {
        **profile.get("options", {}),
        "model": profile["model"],
        "messages": [{"role": "user", "content": prompt}],
        "stream": streaming,
    }
    if streaming:
        payload["stream_options"] = {"include_usage": True}
    headers = {"Content-Type": "application/json", "Accept": "text/event-stream" if streaming else "application/json"}
    if secret:
        headers["Authorization"] = f"Bearer {secret}"
    request = Request(url, data=json.dumps(payload, ensure_ascii=False).encode(), headers=headers, method="POST")
    start = time.perf_counter()
    usage = parse_usage({})
    result = {"state": "outcome_unknown", "content": "", "reported_model": None, "finish_reason": None}
    try:
        direct = not profile.get("use_system_proxy", True)
        if profile.get("http_client") == "curl":
            request_with_curl(url, headers, payload, timeout, direct, result, usage)
        else:
            handlers = [NoRedirect(), ProxyHandler({})] if direct else [NoRedirect()]
            with build_opener(*handlers).open(request, timeout=timeout) as response:
                if streaming and response.headers.get_content_type() == "text/event-stream":
                    read_stream(response, result, usage)
                else:
                    read_json_response(response, result, usage)
    except HTTPError as error:
        with error:
            http_error(result, error.code)
    except (OSError, URLError, HTTPException, subprocess.TimeoutExpired):
        result["error"] = "TRANSPORT"
    except (ValueError, TypeError, KeyError, AttributeError, IndexError, OverflowError):
        result["error"] = "INVALID_RESPONSE"
    finally:
        result.update(usage)
        result.update(price_usage(usage, profile.get("billing_usd_per_million")))
        result["latency_ms"] = round((time.perf_counter() - start) * 1000)
    if secret:
        for key in ("content", "reported_model"):
            if result[key]:
                result[key] = result[key].replace(secret, "[已隐去]")
    return result
