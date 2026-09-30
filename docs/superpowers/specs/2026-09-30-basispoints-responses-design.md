# Basis Points Responses 插件接口设计

## 目标

在现有 `codex-proxy-twofa-plugin` 中增加一个受管理员管理接口保护的 Basis Points Responses 路由。调用方只提交宿主账号 ID 和原始 Responses JSON；插件从宿主受保护的账号导出接口读取该账号的 OAuth `access_token`，向固定的 Basis Points endpoint 发起请求，并返回受限的上游响应。

本功能不改变现有 2FA OAuth 导入、重新授权、凭据保存或任务状态流程，也不新增管理页面。

## 路由合同

新增管理路由：

```text
POST api/basispoints/responses
Content-Type: application/json
```

请求正文严格为：

```json
{
  "accountId": "宿主账号 ID",
  "request": {
    "model": "...",
    "input": "..."
  }
}
```

`accountId` 为非空、有限长度的宿主账号标识。`request` 必须是 JSON object，内部字段由 Basis Points Responses 协议决定，插件不改写其字段。管理请求继承宿主的管理员会话和来源校验；插件不接受客户端提供的目标 URL、代理、Bearer token 或上游认证模式。

响应正文为 Basis Points 返回的原始字节，限制为 2 MiB。允许的响应类型为 `application/json` 和 `text/event-stream`，状态码沿用上游状态码；插件统一添加 `Cache-Control: no-store` 和 `X-Content-Type-Options: nosniff`。当前管理响应仍是有界缓冲，不承诺实时转发 SSE 帧。

## 数据流与组件边界

1. Rust 管理层注册 `api/basispoints/responses`，检查 JSON 内容类型、正文大小和请求路径，然后把会话 Cookie、Origin 和请求正文转交给 Worker
2. Worker 通过管理员会话调用宿主 `GET /api/admin/accounts/export?accountIds=<id>&confirm=export_sensitive_accounts`
3. Worker 展开 `documents[].document.accounts[]`，只接受 provider 为 `openai` 且内部 `id` 与请求 ID 相同的单一账号记录；有 `api_key`、缺少 `access_token` 或多个/不匹配记录时拒绝请求
4. Worker 对 OAuth access token 的 JWT payload 做结构化 Base64URL 解码，从 `https://api.openai.com/auth.chatgpt_account_id` 读取账号 ID；导出账号的 `accountId` 作为已知用户信息优先值，只有两者都缺失时才拒绝请求，claim 与已知账号冲突时拒绝请求
5. Worker 将 `request` JSON 发到固定 URL `https://bps.openai.com/basispoints/api/responses`
6. 请求仅包含固定的 `Authorization: Bearer <access_token>`、`chatgpt-account-id`、`x-openai-account-id`、`x-basispoints-auth-mode: chatgpt` 和 JSON 内容类型。Worker 清理请求完成后的 token 引用，不持久化任何导出内容
7. Worker 返回有限的状态、Content-Type 和正文，Rust 管理层透传允许的响应类型

Rust 层继续负责插件路由和统一管理响应；Node Worker 负责宿主账号导出、JWT claim 解析和外部 HTTP 调用，避免把宿主凭据解密或账号专属逻辑放进插件桥接层。

## 错误与安全边界

- 管理层缺少管理员会话、Origin 校验失败、正文过大或 JSON 结构错误时，在请求到达 Worker 前返回稳定的 4xx 错误
- 宿主导出返回未授权、禁止访问、账号不存在或非 OAuth 账号时，不返回导出文档；只返回面向调用方的短错误
- JWT 不是合法的三段 payload、payload 不是 object、用户信息与 claim 都没有账号 ID，或 claim 与宿主 `accountId` 冲突时返回 400
- Basis Points 非 2xx 响应返回其状态码和有界正文；传输失败或正文超限返回 502/413 类稳定错误，不包含 Authorization 头或 token 内容
- 日志、任务状态、迁移状态、截图和前端响应都不能包含 access token、refresh token、id token 或账号导出文档
- 请求只允许固定 Basis Points URL；不允许 SSRF、用户代理、额外上游头或把 token 转发给其他地址

## 验证

Worker 测试使用本地 mock HTTP server 和虚构 JWT，覆盖：

- 宿主导出查询参数、Cookie 和 Origin
- JWT Base64URL 解码、claim 读取、导出账号匹配和非 OAuth 拒绝
- Basis Points 四个身份头、JSON 请求正文和固定 URL
- 上游 401/403/5xx、无效 JWT、用户信息与 claim 同时缺失、响应类型和 2 MiB 上限
- 错误正文及测试日志不出现任何虚构 token

Rust 测试覆盖路由注册、请求内容类型、路径映射、允许的响应类型和管理响应的 no-store 头。现有 2FA 测试集继续运行，确保新增路由不改变原有流程。

## 非目标

- 不把 access token 暴露给插件前端或客户端
- 不从请求体接受 access token，也不实现 token 刷新或 OAuth 登录
- 不修改宿主 Core、账号导出合同、数据库 schema 或现有 2FA 页面
- 不安装、启用、调用真实 Basis Points 账号或发布插件制品
