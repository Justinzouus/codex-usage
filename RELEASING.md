# 发布 V1.0

## 发布范围

公开仓库从当前 V1.0 源码快照开始，不导入早期 Git 历史、旧 App、迁移说明、真实用量核验记录、账户资料或用户数据。保留两份官方公开价格夹具是为了验证旧格式兼容性，不是旧版本产品存档。

保留 MIT 和 vendor 许可。发布前检查源码、图片、二进制和 DMG 内的实际文件，使用合成数据制作截图。依赖、构建产物和用户本机 `~/.codex`、应用支持目录均不作为仓库内容。

## 检查与构建

```bash
npm ci
npm test
npm run build
npm audit --audit-level=high
cargo test --manifest-path src-tauri/Cargo.toml --locked
npm run tauri -- build --bundles app,dmg
```

本次使用 Gitleaks 扫描公开快照，核对版本、签名、DMG 内容、SHA-256、隔离数据下的原生窗口启动。账期保存、校验与读取已由前端交互和 Rust 持久化测试覆盖；本次原生账期表单交互与重启恢复未完成验收。编译时用 Rust `--remap-path-prefix` 隐去构建者个人路径，并剥离符号。

## 分发

将 DMG、SHA256SUMS.txt 和安装说明上传 GitHub Release `v1.0.0`。README 及网站可链接 Releases；固定版本链接不会随更新改变，`releases/latest` 指向当前正式版。

V1.0 仅提供 macOS Apple Silicon 包，使用 ad hoc 临时签名，尚未完成 Developer ID 签名和 Apple 公证。Release 必须说明架构、前置条件及首次打开限制，不能宣称已公证或已验证所有 macOS 版本。
