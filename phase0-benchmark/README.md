# StoryForge V3 Phase 0

对至少两组 OpenAI 兼容模型做中文盲评。套件包含十个场景、十三个节拍、三个参考字数区间和四份 Writing Skills。每个节拍用传统单 Prompt 和 V3 的隔离上下文分别生成。审查与提取使用八个独立的固定答案案例。

## 环境与启动

从本目录运行，需 Python 3.10 或更新版本。

```powershell
python -m pip install -r requirements.txt
python run_benchmark.py validate
python -m unittest discover -s tests -v
Copy-Item profiles.example.json profiles.local.json
```

修改本机配置的 `base_url` 和 `model`，并在启动终端加载 `api_key_env` 指定的环境变量。服务无需凭据时可将 `api_key_env` 设为 `null`。基地址可包括 `/v1` 或完整的 `/chat/completions`。如中转站要求指定较大的输出上限，可按提供方选择 `options.max_tokens` 或 `options.max_completion_tokens`。

`billing_usd_per_million` 接受 `input`、`output` 和可选的 `cached_input`，价格取自本次实际路由。省略缓存单价时按普通输入单价估算。缓存优惠存在且提供方未给出缓存量时，单次费用留空。未返回的用量以空值保存。

```powershell
python run_benchmark.py plan --config profiles.local.json
python run_benchmark.py run --config profiles.local.json
```

默认两组模型共 172 次调用。遇到失败立即记录该条状态并继续其他独立样本。超时、连接异常或服务端错误记录为 `outcome_unknown`。每条调用只发送一次。若进程被强制结束，可用 `report --run <目录>` 从 SQLite 日志重建已有报告。

## 盲评与校准

评审员使用运行目录中的 `blind-review.md`、`rubric.md`、`skill-review.md` 和两份 CSV 模板。每位评审员复制模板后独立填写。匿名稿按节拍分组，隐藏分组方法、模型和端点。保留 `ledger.sqlite3` 和 `private/` 供跑测人员审核和复核。

```powershell
python run_benchmark.py calibrate --run runs/<运行目录> --ratings ratings.csv --skills skills.csv
```

`private/usage.csv` 按次记录状态、返回的 token 用量、费用估算和耗时。`private/calibration.md` 汇总各模型的通过率、实际篇幅、三角色候选和 Skill 示例得分。`private/exit-gate.json` 记录闸门是否通过。冻结日志保存实际 Prompt、Skill 版本、路由和价格配置。保留原始日志和评分文件即可复算校准。

场景 03 的发送前测试会检查秘密本体和保留词是否出现在林夏的 V3 Prompt 中。回包检查可以捕获已知词形，人工评审负责判断语义层面的泄密。样例单元测试使用临时本机服务，真实模型的使用记录由 `run` 单独生成。

[Chat Completions 请求和用量格式](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create)
