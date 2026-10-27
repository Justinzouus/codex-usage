# Contributing

感谢帮助改进 Codex Usage。

## 开发检查

```bash
npm ci
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml
```

真实额度只在 `npm run tauri dev` 中读取；浏览器预览使用模拟数据。

## 隐私要求

- 不得新增读取 `auth.json`、解析 token 或硬编码私有额度 HTTP 路由的回退路径。
- 不得把原始 app-server 响应、账户信息或 stderr 传到前端或日志。
- 协议解析、格式化和旧数据合并逻辑变更时必须补测试。
- 截图、Issue 和测试夹具必须使用合成数据。
