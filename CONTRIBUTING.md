# Contributing

感谢你对 Agent Meter 的关注。提交改动前，请先创建 Issue 说明问题或方案，避免重复工作。

## 开发环境

- macOS 14 或更高版本
- Node.js 20 或更高版本
- Rust stable
- Xcode Command Line Tools

```bash
npm ci
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

请勿在测试、截图或提交中加入真实会话正文、用户目录、API Key、访问令牌或本机 SQLite 数据库。

提交信息使用简洁的 Conventional Commits 格式，例如 `fix: handle truncated JSONL tail`。
