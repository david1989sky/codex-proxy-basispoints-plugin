# Codex Proxy RS Basis Points 插件

这是一个独立的 Basis Points Responses companion 插件，插件 ID 为
`david1989sky.codex-proxy-basispoints`。它只负责通过 RS 管理会话转发 Responses 请求，不包含账号管理或浏览器授权页面。

## 功能

插件提供受 RS 管理会话保护的 `POST api/basispoints/responses` 接口，请求体为：

```json
{
  "accountId": "宿主账号 ID",
  "request": {
    "model": "模型名",
    "input": "请求内容"
  }
}
```

Worker 从宿主账号导出接口读取指定的 OpenAI OAuth 账号，解析 `accessToken` 和
JWT 的 `claims["https://api.openai.com/auth"].chatgpt_account_id`，再固定请求
[`https://bps.openai.com/basispoints/api/responses`](https://bps.openai.com/basispoints/api/responses)。
调用方不能提交 token、目标 URL 或代理设置。上游 JSON/SSE 状态码和正文会保留，正文上限为 2 MiB。

## 本地验证

环境需要 Rust 1.97 和 Node.js 22 或更高版本：

```bash
npm --prefix worker ci --ignore-scripts --no-audit --no-fund
npm --prefix worker test
cargo +1.97.0 fmt --manifest-path backend/Cargo.toml -- --check
cargo +1.97.0 clippy --manifest-path backend/Cargo.toml --all-targets --all-features --locked -- -D warnings
cargo +1.97.0 test --manifest-path backend/Cargo.toml --locked
```

## 打包

先安装与 RS 兼容的插件 CLI：

```bash
cargo +1.97.0 install --locked --git https://github.com/zyycn/codex-proxy-rs.git \
  --rev 70d1557b55aed871d66dee5db82c2254990d06a6 \
  codex-proxy-plugin-cli --root .tools
PLUGIN_CLI="$PWD/.tools/bin/cpr-plugin" bash scripts/package.sh
```

产物包含独立的 RS 插件归档、Worker bundle、摘要清单和 SHA-256 文件。

## 安装

在 RS 的「插件管理」上传 BPS 插件归档，确认插件 ID 为
`david1989sky.codex-proxy-basispoints`。Worker 使用
`http://127.0.0.1:28082`，可用 [`ops/companion-install.sh`](ops/companion-install.sh)
和 [`ops/companion-update.sh`](ops/companion-update.sh) 部署固定摘要的镜像。

插件只接受本机 Worker 地址，并通过 RS 管理会话转发请求。Worker 为无状态服务，不挂载凭据目录或密钥文件。
