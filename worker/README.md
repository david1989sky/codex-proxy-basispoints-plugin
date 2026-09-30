# Basis Points Worker

这个 Worker 只负责 Basis Points Responses 请求转发，不启动浏览器、不保存账号凭据，也不提供账号管理页面。

## 环境变量

- `PUBLIC_ORIGIN`：RS 公共来源，用于同源校验。
- `CPR_BASE_URL`：Codex Proxy RS 地址，默认 `http://127.0.0.1:28080`。
- `PORT`：监听端口，默认 `28082`。

## 接口

- `GET /health`：返回 Worker 就绪状态。
- `POST /api/basispoints/responses`：插件内部调用需要 `X-CPR-Basispoints: 1`、
  `X-CPR-Basispoints-Internal: 1`，请求体为 `{accountId, request, accessToken}`；兼容带 RS 会话的旧调用路径。

Worker 接收插件通过宿主回调解析出的 OAuth `accessToken`，然后只请求固定的
`https://bps.openai.com/basispoints/api/responses`。支持的上游类型为
`application/json` 和 `text/event-stream`，响应正文限制为 2 MiB。转发前会补齐 BPS 所需的
请求字段和 Excel 客户端请求头，同时保留输入历史、工具目录及上下文字段。
