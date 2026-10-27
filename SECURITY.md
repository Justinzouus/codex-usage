# Security

## 安全边界

- 不读取、不复制、不持久化 Codex 凭据。
- 直接启动 `codex`，不经过 shell；可用 `CODEX_CLI_PATH` 显式指定可信二进制。
- app-server 的每一行输出最多 1 MiB，超过即拒绝。
- 初始化与额度读取分别有 5 秒和 12 秒超时。
- 不记录原始协议消息或 Codex stderr。
- 前端只接收脱敏后的额度快照，CSP 禁止任意网络连接。
- 只调用只读的 `account/rateLimits/read`，不兑换重置机会，也不修改账户。

## 兼容性边界

`codex app-server` 命令目前标记为 experimental，但 `account/rateLimits/read` 已存在于非实验协议 Schema。若方法、字段或启动参数变化，应用会提示更新 Codex，不会退回到读取凭据或调用硬编码私有路由。

## 报告问题

请勿在公开问题中粘贴 token、账户 ID、原始协议响应、本机路径或带个人数据的截图。即使应用不记录这些内容，手工诊断资料仍应先脱敏。

## 发布

V1.0 的 macOS Apple Silicon 包在经过脱敏筛选的发布快照中构建，使用 ad hoc 临时签名，尚无 Developer ID 签名或 notarization。SHA-256 用于核对文件一致性，不能替代发布者身份认证。安装限制在 README 与 Release 中明确说明。
