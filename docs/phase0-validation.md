# Phase 0 验证记录

本机使用 Python 3.11.15 与 PyYAML 6.0.3。从仓库根目录运行下列命令。

```powershell
python -X utf8 -m unittest discover -s phase0-benchmark/tests -v
python -X utf8 phase0-benchmark/run_benchmark.py validate
python -X utf8 phase0-benchmark/run_benchmark.py plan --config phase0-benchmark/profiles.example.json
```

| 验证项 | 结果 |
| --- | --- |
| 单元与本机 HTTP 集成测试 | 8 项通过 |
| 场景、节拍、Skill 与角色案例 | 10 个场景、13 个节拍、4 项 Skill、8 个审查与提取案例通过校验 |
| 三个参考篇幅下的双方法矩阵 | 两组配置共计划 172 次调用 |
| 林夏 POV | 三个篇幅下均剥离秘密本体，保留可见线索。污染文本在请求前被拒绝 |
| HTTP、用量与匿名 | 本机测试验证了请求格式、断连记录、拒绝重定向、同一条调用不重发、缓存用量缺失值、费用计算及模型名和凭据脱敏 |
| 校准验收逻辑 | 缺失人工评分或对抗场景出现语义泄密标记时，验收返回未通过 |

## 待验收

当前运行环境缺少获准调用的真实模型配置。真实用量账单、角色路由组合、模型篇幅校准、示例盲评及真实正文的语义泄密检查均待完成。本地集成测试使用临时回环服务和测试数据，数据随测试清理。
