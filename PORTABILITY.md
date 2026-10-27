# V1.0 迁移与恢复

核验日期：2026-10-27。公开仓库仅包含 V1.0 当前源码快照，不含作者本机迁移历史与用户数据。

## 当前状态

macOS Apple Silicon 应用，版本号 1.0.0，产品名 Codex Usage。开发环境见 README，构建与发布见 RELEASING.md。保留 vendor 菜单栏补丁及其版权许可。

## 外部数据

本机 Codex 历史、项目元数据与认证均不随源码或 DMG 分发。应用自己的 preferences.json、billing-settings.json、api-pricing-cache.json、service-mode-evidence.json 保存在用户的 Library/Application Support/app.codexusage.widget/。其中个人账期和模式证据属于私有数据。

迁移时先退出应用，对项目源码和未提交文件另做备份，再保存应用支持目录及所需的原始 Codex 历史。数据库应按一致快照备份；不能只依靠 Git 或可重建构建产物恢复用户数据。不要将凭据放入公开仓库。

## 恢复核验

在新设备安装并登录 Codex，恢复需要的本地历史和应用设置，启动 Codex Usage，确认版本、额度、项目归属、历史覆盖与个人账期。公开价格缓存可以重建，历史与模式证据不能靠重新构建 App 恢复。
