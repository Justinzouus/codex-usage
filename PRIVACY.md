# Privacy

Codex Usage 的目标是让额度可见，同时不扩大本机凭据的暴露面。

## 应用读取什么

- 应用查找并启动本机 `codex` 可执行文件。
- 应用通过 Codex app-server 的 `account/rateLimits/read` 方法读取只读额度快照。
- 应用不会打开 `.codex/auth.json`，不会读取 API key、access token、refresh token 或账户 ID。
- 使用大盘只读 `$CODEX_HOME/sessions`、`archived_sessions` 和 `session_index.jsonl`（默认 `~/.codex`），提取时间、模型、Token、执行/父子 ID、工作目录、额度窗口和最多 90 字的用户执行短标题或会话标题。
- 明细筛选额外只读 `.codex-global-state.json` 的项目名称、目录、对话项目分配与独立对话列表；其他字段不反序列化、不保留、不上传。
- 流式读取日志会经过同文件中的其他字节，但不保留完整提示词、回复、工具结果或原始记录；不跟随历史目录内符号链接。
- 原生验收专用的 `CODEX_USAGE_HISTORY_DIR` / `CODEX_USAGE_STATE_DIR` 必须成对提供绝对路径，分别隔离历史读取及偏好/价格缓存；不改变 Codex 的登录目录。未设置时沿用原有数据位置。
- 为恢复真实速度模式，只读 `logs_2.sqlite` 的 `logs` 表中 `feedback_tags` / `tags_json` 诊断记录，精确关联会话 ID、轮次 ID、模型和请求模式。通过系统 SQLite 的只读连接执行固定 SELECT；不读取其他 Codex 数据库。临时解析诊断正文后只保留这几个模式字段，不保存诊断正文或聊天内容，不向外发送。

## 应用发送什么

Codex Usage 为自动更新 API 单价，会通过系统 curl 读取 `https://developers.openai.com/api/docs/pricing.md`，并在需要时查询该官方站点的模型文档。请求不携带登录凭据、Token 用量、标题、聊天记录或本机路径；单模型补查的 URL 只包含模型名称。无需 API key，不调用付费模型或第三方搜索服务。

额度由本机 Codex 子进程使用自己的登录态获取；子进程关闭 plugins、remote_plugin、plugin_sharing、apps，并设置 `analytics.enabled=false`。

## 应用保存什么

- 语言、置顶、锁定、固定服务和轮换间隔等悬浮窗偏好。
- 使用者自行填写的订阅费、扣款日、时区和额外额度金额保存为本机 `billing-settings.json`，macOS 权限0600；新安装无预设个人账期，设置不会发送到服务器。
- 统计缓存只在进程内存中，退出即释放；不额外持久化执行标题或历史日志。
- 官方公开价格及同步状态保存在应用数据目录的价格缓存中，联网失败时保留上次有效价格。这与本机用量历史分开保存。
- `service-mode-evidence.json` 保存精简的会话/轮次/模型/模式证据及读取位置，供诊断日志清理后继续识别历史用量；权限为仅当前用户读写。该文件不含 Token、标题、提示词、认证信息或诊断正文。

额度快照和历史统计只保存在进程内存中，不写入磁盘。应用不保存 Codex 凭据、账户 ID、原始协议响应、完整提示词或聊天记录。

## 日志

日志必须保持通用，不得包含原始 app-server 行、Codex stderr、账户信息、本机认证路径或个人文件路径。

## 准确性

应用显示 Codex 返回的额度窗口，不根据本地 token 统计估算额度。如果协议或数据结构变化，应用会显示不可用或旧数据状态，而不会编造数值。

大盘只代表本机可读历史；未定价模型、不可读文件、父子关联缺口和推定重置周期均明确注明。API 等价费用和估算差额不是账单、退款或实际收益。详见 `docs/ANALYTICS.md`。
