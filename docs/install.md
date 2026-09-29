# 安装与更新

目标服务器需要 Codex Proxy RS `3.18.1` 或更高的 `3.x` 版本，平台为 `x86_64-unknown-linux-gnu`。插件包和 companion Worker 镜像必须来自同一个 GitHub Release。

## v0.1.0 发布值

公开 Release：[github.com/david1989sky/codex-proxy-twofa-plugin/releases/tag/v0.1.0](https://github.com/david1989sky/codex-proxy-twofa-plugin/releases/tag/v0.1.0)

```text
插件 SHA-256  9b54b450dff49a1bc563e6879ae8bb0af62726e1b5c6c93fe8e704b2b48b8185
Worker bundle SHA-256  e46efabf8ea164f3d77b5fb933784b031e09d6bb02c1e1c4ee98095d5694f970
Worker image  ghcr.io/david1989sky/codex-proxy-twofa-worker:v0.1.0@sha256:c2ac501b6e5892b42a2caba35a2c681d866b1a2863887bb7b82b9b532596158e
```

## 首次安装

先下载 Release 中的插件归档、对应 `.sha256`、Worker bundle、bundle `.sha256`、`companion-manifest.json` 和两个 companion 脚本。核对摘要：

```bash
sha256sum -c david1989sky.codex-proxy-twofa-0.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256
sha256sum -c codex-proxy-twofa-worker-0.1.0.tar.gz.sha256
```

从清单取出固定的镜像摘要，然后在 RS 主机执行：

```bash
export PUBLIC_ORIGIN=https://cx.subarx.com
export WORKER_IMAGE=ghcr.io/david1989sky/codex-proxy-twofa-worker:v0.1.0@sha256:c2ac501b6e5892b42a2caba35a2c681d866b1a2863887bb7b82b9b532596158e
export WORKER_BUNDLE=/path/to/codex-proxy-twofa-worker-0.1.0.tar.gz
sudo -E bash companion-install.sh
```

脚本会把 Worker 放到 `/opt/cpr-twofa/release/codex-proxy-twofa-worker`，使用现有的 `/opt/cpr-twofa/data/credentials` 和 `/opt/cpr-twofa/secrets/twofa-key` 挂载，并等待 `http://127.0.0.1:28082/health` 返回 `ready=true`。空凭据目录首次安装时才会生成密钥；已有数据但缺少密钥会停止部署。

Worker 就绪后，在 RS「插件管理」上传插件归档，核对安装包摘要，确认完整信任并启用。插件配置填写：

```text
workerBaseUrl = http://127.0.0.1:28082
workerImageDigest = sha256:c2ac501b6e5892b42a2caba35a2c681d866b1a2863887bb7b82b9b532596158e
```

打开「批量 2FA 授权」页面，刷新状态并执行一次「确认迁移」。迁移只写入 RS 插件状态，不读取或移动加密凭据。

## 更新

每次更新先备份当前 Release 目录和加密凭据目录，再下载新版本的 bundle 和镜像摘要：

```bash
export PUBLIC_ORIGIN=https://cx.subarx.com
export WORKER_IMAGE=ghcr.io/david1989sky/codex-proxy-twofa-worker:v0.2.0@sha256:<新版本 digest>
sudo -E bash companion-update.sh
```

脚本会把当前镜像摘要保存到 `/opt/cpr-twofa/backup/twofa-worker/previous-image`，启动新镜像并执行健康检查。RS 插件包随后在「插件管理」中上传更新；启用前确认它的 `workerImageDigest` 与 Worker 清单一致。

## 回滚

Worker 健康检查失败时，旧镜像仍保留在本机。执行：

```bash
sudo bash companion-update.sh --rollback
```

这只回滚 companion Worker。RS 核心容器、PostgreSQL、Redis 和加密凭据不会被脚本触碰。插件包回滚使用 RS「插件管理」中的已安装版本记录；回滚后再次检查插件页、Worker 状态和迁移标记。

## 卸载

先在 RS 中停用并卸载插件，再停止 companion Worker：

```bash
docker compose -p cpr-twofa -f /opt/cpr-twofa/release/codex-proxy-twofa-worker/ops/compose.yaml down
```

加密凭据目录和密钥不会自动删除。确认不再需要这些账号后，按组织的备份和销毁流程处理 `/opt/cpr-twofa/data/credentials` 与 `/opt/cpr-twofa/secrets/twofa-key`。
