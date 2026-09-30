# Basis Points Worker

这个 Worker 只负责 Basis Points Responses 请求转发，不启动浏览器、不保存账号凭据，也不提供账号管理页面。

## 环境变量

- `PUBLIC_ORIGIN`：RS 公共来源，用于同源校验。
- `CPR_BASE_URL`：Codex Proxy RS 地址，默认 `http://127.0.0.1:28080`。
- `PORT`：监听端口，默认 `28082`。

## 接口

- `GET /health`：返回 Worker 就绪状态。
- `POST /api/basispoints/responses`：需要 RS 管理会话、来源校验和 `X-CPR-Basispoints: 1`，请求体为 `{accountId, request}`。

Worker 通过 RS 的账号导出接口读取 OAuth `accessToken`，然后只请求固定的
`https://bps.openai.com/basispoints/api/responses`。支持的上游类型为
`application/json` 和 `text/event-stream`，响应正文限制为 2 MiB。
