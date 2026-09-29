# Codex Proxy RS 批量 2FA 插件

这个仓库把批量 2FA 导入、浏览器授权、人工验证接管和已保存账号重新授权整理为 Codex Proxy RS 插件。插件管理页运行在 RS 的 `/plugins` 页面中，授权浏览器由独立的 companion Worker 负责。

## 发布内容

每个 GitHub Release 包含：

- 面向 `x86_64-unknown-linux-gnu` 的 RS 插件安装包和 SHA-256 文件
- 固定摘要的 companion Worker 镜像清单
- companion Worker 安装、更新和回滚脚本

插件要求 Codex Proxy RS `>=3.18.1, <4.0.0`。插件进程只接受本机 HTTP Worker 地址，并通过 RS 管理会话转发请求；Worker 继续使用现有 AES-256-GCM 加密凭据目录和密钥挂载。

## 本地检查

环境需要 Rust 1.97、Node.js 24 和 pnpm。执行：

```bash
pnpm --dir frontend install --frozen-lockfile
pnpm --dir frontend run typecheck
pnpm --dir frontend run lint
pnpm --dir frontend run build
npm --prefix worker ci --ignore-scripts --no-audit --no-fund
npm --prefix worker test
cargo +1.97.0 fmt --manifest-path backend/Cargo.toml -- --check
cargo +1.97.0 clippy --manifest-path backend/Cargo.toml --all-targets --all-features --locked -- -D warnings
cargo +1.97.0 test --manifest-path backend/Cargo.toml --locked
```

打包前安装与 RS v3.18.1 相同提交的 CLI：

```bash
cargo +1.97.0 install --locked --git https://github.com/zyycn/codex-proxy-rs.git \
  --rev 70d1557b55aed871d66dee5db82c2254990d06a6 \
  codex-proxy-plugin-cli --root .tools
PLUGIN_CLI="$PWD/.tools/bin/cpr-plugin" bash scripts/package.sh
```

## 安装到 RS

1. 在 GitHub Release 下载与服务器平台匹配的插件归档和 `.sha256` 文件。
2. 在 RS 的「插件管理」上传归档，核对摘要并确认信任。
3. 先按 `docs/install.md` 安装固定摘要的 companion Worker。
4. 启用插件，确认 Worker 地址为 `http://127.0.0.1:28082`，填入已核对的 Worker 镜像摘要。
5. 打开「批量 2FA 授权」页面，检查 Worker 状态并执行一次迁移标记确认。

插件安装代表信任插件进程。RS 插件机制不提供操作系统级沙箱；插件不会读取 PostgreSQL，也不会把密码、TOTP、密钥或真实截图提交到 GitHub。

详细更新、备份和回滚步骤见 [docs/install.md](docs/install.md)，迁移语义见 [docs/migration.md](docs/migration.md)。
