# Agent Meter

[![CI](https://github.com/leoyoul/agent-meter/actions/workflows/ci.yml/badge.svg)](https://github.com/leoyoul/agent-meter/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-2ea44f.svg)](LICENSE)

Agent Meter 是一个本地优先的 macOS 菜单栏应用，用来查看 Codex 主代理和子代理的 Token 用量、首响、有效 TPS 与任务耗时。

> Agent Meter 是社区维护的非官方工具，与 OpenAI 无隶属或背书关系。

![Agent Meter dashboard](docs/dashboard.png)

## 功能

- 菜单栏以独立紧凑状态项显示今日 Token、最近 5 个有效完成任务的首响中位数和有效 TPS 中位数，每项均可单独开关。
- 按日期、数据源、模型、项目和 Agent 类型筛选。
- 查看 Token 趋势、模型 P50/P95、主子代理任务树和最近任务。
- 首次流式索引历史 JSONL，随后通过文件监听增量更新，并以低频扫描兜底。
- 支持暂停、断点续扫、数据源启停、重建索引和可选的登录启动。
- 启动后自动检查稳定版更新，经用户确认后下载、验证、安装并重启。

## 隐私边界

- 数据源只读，不修改 Codex 会话文件。
- 不连接 sub2api、OpenAI Admin API 或其他外部服务。
- 不保存提示词、回复正文、工具参数或工具输出。
- 不显示或估算金额。
- SQLite 仅保存统计所需的会话关系、模型、时间、项目和数值指标。

## 指标口径

- 今日 Token：按本地自然日汇总输入和输出 Token；缓存输入是输入 Token 子集，推理 Token 是输出 Token 子集，均不重复计入总量。
- 首响：Codex `task_complete.time_to_first_token_ms`。
- 整轮耗时：Codex `task_complete.duration_ms`。
- 有效 TPS：`output_tokens / ((duration_ms - time_to_first_token_ms) / 1000)`。

有效 TPS 包含思考、工具等待和同一轮内的多次模型调用，不能理解为模型服务端的纯解码速度。

## 安装

从 [Releases](https://github.com/leoyoul/agent-meter/releases) 下载 Apple Silicon DMG。`v0.2.0` 起支持应用内更新；`v0.1.0` 用户需要手动安装一次新版。DMG 尚未经过 Apple Developer ID 签名或公证，macOS 可能阻止直接启动；对供应链安全有要求时，请审查源码并自行构建。

应用内更新包使用 Tauri 更新签名验证完整性。该签名不等同于 Apple Developer ID 签名或 Apple 公证。

## 数据源

应用自动发现存在的 `~/.codex`、`~/.yodex` 和 `~/.lodex`，并扫描其中名称为 `rollout-*.jsonl` 的会话文件。每个数据源可在设置中独立启停。

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
npm run tauri -- build --bundles app,dmg
```

浏览器开发模式使用完全脱敏的模拟数据，默认地址为 `http://127.0.0.1:4319`。

## 参与贡献

请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。安全问题请按 [SECURITY.md](SECURITY.md) 私密报告。

## License

[MIT](LICENSE) © 2026 LeoY
