# Agent Meter

[![CI](https://github.com/leoyoul/agent-meter/actions/workflows/ci.yml/badge.svg)](https://github.com/leoyoul/agent-meter/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-2ea44f.svg)](LICENSE)

Agent Meter 是一个本地优先的 macOS 菜单栏应用，用“速、首、量、费”观察 Codex、ZCode、OpenCode、DSH 和 EvoX 的本机模型调用，并检测 Claude Desktop 的本地数据能力。

> Agent Meter 是社区维护的非官方工具，与 OpenAI、Anthropic 及所支持 Agent 的开发者无隶属或背书关系。

![Agent Meter dashboard](docs/dashboard.png)

## 功能

- 菜单栏提供四个独立的 `30×22pt` 双行状态项，大数值在上、指标名在下，每项固定宽度并可单独开关；应用图标默认隐藏，可在设置中打开。
- 仪表盘使用顶层来源标签；当前来源同步用于仪表盘和菜单栏，重启后保持选择。
- 仪表盘和设置是标准 macOS 窗口，打开时出现在 Dock 与 `⌘Tab`，关闭后继续在菜单栏后台运行。
- 实时、今日、本周、本月和本年五个统一统计周期。
- 按来源、模型和推理强度查看有效 TPS、首响、Token 与 API 等价费用。
- 每个完成且 Token 大于零的模型响应按稳定调用 ID 单独计数；运行中任务已经产生的响应也会立即进入用量统计。
- 仪表盘提供统计可信度对账状态，并在“价格”页管理所有历史使用模型的价格版本。
- Token 使用互不重叠的非缓存输入、缓存读取、缓存写入和输出计量桶；推理 Token 是输出子集。
- Codex 使用流式 JSONL 增量索引，ZCode 与 OpenCode 只读其 SQLite 数据库；DSH 流式解压 JSONL zstd，EvoX 只读 observability JSONL。
- 支持暂停、断点续扫、数据源启停、重建 Codex 索引、登录启动和应用内稳定版更新。

## 指标口径

- `调用`：一条完成且 Token 大于零的模型响应算一次；Codex 使用唯一 `response_id`，不再用 turn 数代替调用数。
- `速`：单个可靠性能样本为 `output_tokens / ((duration_ms - ttft_ms) / 1000)`，多样本取 Token 加权平均；解码窗口小于 500ms 的异常样本不参与统计。性能样本数与调用数独立展示。
- `首`：开始到首个内容 Token 的时间。缺少首内容时间的历史记录不参与平均，不用总耗时代替。
- `量`：`非缓存输入 + 缓存读取 + 缓存写入 + 输出`。推理 Token 不重复计数。
- `费`：按调用日期对应的厂商官方标准文本 API 单价计算，使用整数 nano-USD 汇总。

实时周期的调用、量和费取最近 10 次模型响应，速和首独立取最近 10 个可靠性能样本。其他周期按每次调用自己的发生时间转换到本机时区，以自然日、周一、月初和年初为边界。

费用是 API 等价估算，不是 Codex 订阅、第三方套餐或实际账单。内置价目首次迁入本机 SQLite 后可直接增删改查，并按生效日期保留历史版本；删除预置版本后升级不会自动恢复。缓存价格空白表示该桶未计价，显式 `0` 表示免费。未知价格不会按零处理，而是显示已知费用和计价覆盖率。当前目录的官方来源包括 [OpenAI GPT-6 Astra 模型文档](https://developers.openai.com/api/docs/models/gpt-6-astra)、[OpenAI 模型文档](https://developers.openai.com/api/docs/models/gpt-5.6-sol)、[Vercel AI Gateway Muse Spark 1.3 公告](https://vercel.com/changelog/muse-spark-1-3-now-available-on-ai-gateway)、[Muse Spark 1.2 Contributor](https://vercel.com/ai-gateway/models/muse-spark-1.2-contributor/about)、[Xiaomi MiMo V2.5](https://vercel.com/ai-gateway/models/mimo-v2.5/about)、[Tencent HY3](https://vercel.com/ai-gateway/models/hy3/about)、[Z.AI GLM-5.3 Flash](https://vercel.com/ai-gateway/models/glm-5.3-flash)、[MiniMax 按量价格](https://platform.minimax.io/docs/guides/pricing-paygo) 和 [DeepSeek 定价](https://api-docs.deepseek.com/quick_start/pricing)。

## 隐私边界

- 数据源只读；应用只写自己的 SQLite、设置和诊断日志。
- 不上传会话数据，不连接 Admin API、代理服务或任何统计后端。
- 不保存提示词、回复正文、工具参数或工具输出。
- 只计算标准文本 Token API 等价费用；图片、工具调用、订阅额度和汇率不纳入。

## 数据源

| 来源 | 只读位置 | 增量方式 |
| --- | --- | --- |
| Codex | `~/.codex/sessions`、`~/.codex/archived_sessions` | 文件偏移与监听 |
| ZCode | `~/.zcode/cli/db/db.sqlite` 的 `model_usage` | 最近调用重叠同步 |
| OpenCode | `~/.local/share/opencode/opencode.db` | `time_updated + id` 游标 |
| DSH | `~/.dsh/sessions/**/*.jsonl.zstd` | 压缩文件大小与修改时间 |
| Claude Desktop | `~/Library/Application Support/Claude` | 仅检测；未发现稳定的本地 Token 日志 |
| EvoX | `~/.evox/agent/observability/*.jsonl` | 文件变化与稳定调用 ID 去重 |

应用不会扫描 ZCode 的完整目录，也不会读取 Claude 的 IndexedDB、Cache、Cookies 或对话正文。用量调用与性能样本分表保存，Codex 活动目录与归档目录继续按 `response_id` 去重。

## 安装

从 [Releases](https://github.com/leoyoul/agent-meter/releases) 下载 Apple Silicon DMG。`v0.2.0` 起支持应用内更新。DMG 尚未经过 Apple Developer ID 签名或公证，macOS 可能阻止直接启动；对供应链安全有要求时，请审查源码并自行构建。

应用内更新包使用 Tauri 更新签名验证完整性。该签名不等同于 Apple Developer ID 签名或 Apple 公证。

## 从源码构建

需要 macOS 14+、Node.js 20+、Rust stable 和 Xcode Command Line Tools。

```bash
git clone https://github.com/leoyoul/agent-meter.git
cd agent-meter
npm ci
npm run desktop
```

运行检查和构建 ARM64 App/DMG：

```bash
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
npm run tauri -- build --target aarch64-apple-darwin
```

浏览器开发模式使用脱敏模拟数据，默认地址为 `http://127.0.0.1:4319`。

## 参与贡献

请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。安全问题请按 [SECURITY.md](SECURITY.md) 私密报告。

## License

[MIT](LICENSE) © 2026 LeoY

### Claude Desktop 数据边界

当前检查的 Claude Desktop 版本未发现可验证的公开 Token 用量记录，因此仍标记为不可统计。应用不会读取 IndexedDB、Cache、Cookies、会话存储或对话正文，也不会从字段字符串推断用量或拦截网络请求。Claude Desktop Token 解析器尚未实现；本版本不宣称支持其 Token 统计。
