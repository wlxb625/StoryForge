# StoryForge V3

面向中文长篇小说的 AI 创作工作台，按 V3 工程契约的六个垂直切片开发。

## 开发进度

Phase 0 至 Phase 4 已通过阶段验收。样本覆盖和角色选型见 [实测校准](docs/adr-001-phase0-calibration.md)。Windows 运行与测试见 [桌面端验收](docs/phase1-validation.md)。专项结果见 [长期记忆](docs/phase3-validation.md) 和 [改纲与精修](docs/phase4-validation.md)。

Phase 5 已实现全景、后台连写和文件导出。本机测试、Windows 打包和待独立完成的验收项见 [第五阶段交付与验收](docs/phase5-validation.md)。

| 阶段 | 目标 |
| --- | --- |
| 0 | 用真实模型校准节拍长度、主笔与审查提取路由、首批 Writing Skills |
| 1 | 桌面工程文件、模型凭据、单节拍流式生成和不可变修订 |
| 2 | 场景意图、Writing Skills、流派卡带和 A/B 盲测实验室 |
| 3 | Canon、证据化记忆、POV 过滤和中文检索 |
| 4 | 改纲差分、局部精修和按需文学诊断 |
| 5 | 全景库、后台批量续写、快照及成稿导出、Windows 分发和长程压测 |

## Windows 构建

安装依赖后运行 `pnpm tauri dev` 打开开发版。运行 `./scripts/package-windows.ps1` 将 x64 安装包和便携版输出至 `releases/`。
