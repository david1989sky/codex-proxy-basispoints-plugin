# Codex Proxy RS 2FA 插件设计

## 目标

将当前生产环境中的批量 2FA 导入、自动 OAuth 授权、人工验证、重新授权和加密凭据保存功能，整理为可独立安装的 Codex Proxy RS 插件。插件只面向生产服务器平台 `x86_64-unknown-linux-gnu`，通过公开 GitHub 仓库 `david1989sky/codex-proxy-twofa-plugin` 的稳定 Release 分发，并由 RS 插件管理页负责安装、启用、更新、切换和回滚。

## 运行架构

插件包由四部分组成：

1. Rust 插件宿主 `cpr-twofa-plugin` 使用官方 `gateway-plugin-sdk` 注册管理路由、隔离管理页面、日志和私有状态，并负责 Worker 的生命周期
2. Node/Playwright Worker 保留当前已经验证的授权流程，包括批量任务、人工验证、重试、取消、账号关联和加密凭据保存
3. Vue 管理页面运行在 RS 提供的隔离 iframe 中，只通过 `window.codexProxyPlugin.request` 调用插件管理路由，并继承宿主主题
4. `plugin.json`、资源映射和 GitHub Actions 发布脚本负责声明兼容范围、打包资源、生成安装归档与 SHA-256 校验文件

插件主进程以插件包目录为当前工作目录，通过固定的插件资源路径启动 Worker。Node 运行时、Playwright 依赖和 Chromium 运行资源随 `x86_64-unknown-linux-gnu` 包固定发布，避免生产主机环境变化导致授权失败。插件数据不写入临时制品目录，使用稳定的插件私有数据目录；制品切换不会丢失 Worker 状态。

插件不修改宿主 Core、Admin 或 Store，也不直接访问 PostgreSQL。账号、密钥、模型和私有状态通过官方 SDK 回调访问；插件安装即表示安装者信任插件进程的系统权限，插件机制不提供 OS 沙箱。

## 管理接口与页面

管理桥提供与现有功能等价的接口，所有响应正文由插件定义：

- `GET /status`：返回 Worker 就绪状态、迁移状态和当前任务数量，不返回凭据
- `GET /accounts/:accountId/credentials`：返回是否已保存及更新时间
- `DELETE /accounts/:accountId/credentials`：清除对应加密凭据
- `POST /accounts/:accountId/reauthorize`：使用已保存内容或一次性导入内容创建重新授权任务
- `POST /tasks`：创建批量 2FA 任务
- `GET /tasks/:taskId`：读取任务进度、脱敏账号状态和失败信息
- `GET /tasks/:taskId/items/:itemId/screen`：获取人工验证截图
- `POST /tasks/:taskId/items/:itemId/input`：提交人工验证输入
- `POST /tasks/:taskId/cancel`、`POST /tasks/:taskId/retry`：控制任务
- `GET /migration`、`POST /migration/import`：检查和执行旧 Worker 数据迁移

管理页面包含批量导入、任务进度、人工验证、已保存凭据状态、重新授权和迁移状态。页面不能读取管理 Cookie、插件密钥、任意 URL 或宿主 Vue 实例；静态资源和页面入口都由插件清单声明。

## 数据、凭据与迁移

邮箱密码和 TOTP 密钥继续使用 AES-256-GCM 加密。密钥与密文分离保存；任何管理响应、日志、截图、GitHub 文件和插件清单都不包含明文凭据。

首次启用时执行一次性迁移：读取旧 Worker 的加密凭据目录和密钥，校验格式与权限，将记录导入插件私有数据；迁移标记包含源版本、记录数量和完成时间，重复执行不会覆盖已存在的新记录。迁移失败保持旧数据不变，页面显示可重试的错误。

任务元数据和迁移标记使用插件声明的私有状态命名空间，并以精确版本更新避免并发覆盖。加密凭据使用插件私有加密存储，更新和回滚只切换代码与配置，不删除凭据。停用保留所有数据；删除配置或卸载前明确告知删除范围。

## 生命周期、更新与回滚

插件安装后由宿主准备默认配置和资源，插件主进程负责启动 Worker 并报告就绪状态。停用时先停止接受新任务，再取消或排空运行任务，最后关闭 Worker。Worker 异常退出只影响插件实例，RS 其他转发和管理功能继续运行。

GitHub 更新流程为：检查稳定 Release、下载并校验 `.tar.gz` 与 `.sha256`、解析插件清单、确认兼容范围、保留当前配置和状态、切换新制品、启动新 Worker。新版本启动或迁移失败时保留旧实例和旧数据，并允许从已安装版本回滚。版本号使用明确的新版本，不覆盖已发布制品。

## 工程结构

```text
plugin.json
backend/
  Cargo.toml
  src/
worker/
  src/
  test/
  package.json
frontend/
  src/
  package.json
scripts/
  build-linux-x64.sh
  package.sh
docs/
  install.md
  migration.md
.github/workflows/release.yml
```

插件后台只依赖公开 SDK，不从宿主源码跨仓导入。Worker 的管理协议由 Rust 桥接层统一封装，前端不直接依赖旧 `/api/admin/twofa` 路由。发布脚本调用目标版本的 `cpr-plugin package`，不把源码归档或页面目录误当安装包。

## 验证与交付

本地验证包括：

- `cargo fmt --check`、`cargo clippy --all-targets --all-features --locked`、`cargo test --locked`
- 前端 `pnpm install --frozen-lockfile`、`pnpm run typecheck`、`pnpm run lint`、`pnpm run build`
- Worker 的单元测试和不使用真实凭据的浏览器流程测试
- 插件 CLI 清单校验、目标平台打包、包内文件清单和 SHA-256 校验
- 本地 RS `3.18.1` 安装测试：安装、启用、管理页面、迁移、任务控制、停用、更新和回滚

生产验证只使用用户已有授权和虚构测试输入，不输出真实凭据。发布完成后从 GitHub 公开 Release 下载附件重新校验，再在生产 RS 插件管理中安装；插件真实生效必须以管理页面、Worker 就绪、迁移结果和一条受控授权任务共同确认。

## 非目标

- 首发不支持 ARM 或 macOS
- 不修改官方 RS 核心前端、核心 API 或数据库 schema
- 不绕过人工验证、不自动重试账号 401、不保存失败账号的明文凭据
- 不把私有 GitHub Token、管理员密码、代理认证或真实 2FA 数据放入仓库、Release 或 CI 日志
