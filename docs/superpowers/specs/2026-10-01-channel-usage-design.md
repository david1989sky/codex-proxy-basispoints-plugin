# BPS 通道使用情况设计

## 目标

在 BPS 插件的 RS 管理扩展页显示当前插件进程的通道请求使用情况：

- 请求总数
- 成功请求数
- 失败请求数
- 最近请求时间

页面只展示聚合数字，不展示请求正文、响应正文、账号标识、access token 或 Token 内容。

## 统计口径

统计只覆盖真正命中 BPS request middleware 并准备调用 BPS 上游的请求。未命中模型列表、非 OpenAI 请求和直接交给 RS 原生 Provider 的请求不计入。

一次 BPS 上游调用在发起前增加总数并记录当前时间。收到 HTTP 2xx 响应记为成功；非 2xx 响应、请求传输错误、响应解析错误和 403 回退记为失败。403 回退仍然由 RS 原生 Provider 继续处理，但该 BPS 通道尝试本身属于失败。

管理接口返回以下 JSON 字段：

```json
{
  "totalRequests": 0,
  "successfulRequests": 0,
  "failedRequests": 0,
  "lastRequestAtMs": null
}
```

`lastRequestAtMs` 使用 Unix 毫秒时间戳，页面负责按本地时区格式化；没有请求时为 `null`。

## 架构

`PluginState` 增加共享的 `Arc<UsageStats>`。中间件和管理处理器继续使用同一个状态实例：

1. middleware 在调用 `basispoints::request` 前记录一次开始和时间。
2. 收到响应或错误后记录成功或失败。
3. `GET api/usage` 读取原子计数快照并返回 JSON。

计数使用 `AtomicU64`，读取不阻塞请求；计数只保存在插件进程内，插件重启、升级或重新加载后从零开始。此版本不把每个请求写入 RS state，避免高频 IPC 和并发版本冲突。

## 管理扩展页

插件注册一个 `usage` 页面，并声明三个包内资源：`web/index.html`、`web/app.js`、`web/app.css`。页面通过 RS 注入的 `window.codexProxyPlugin.request` 调用相对管理路由，不直接访问网络。

页面包含：

- 标题“BPS 通道使用情况”
- 四个统计值
- “刷新”按钮
- 最近更新时间和统计范围提示
- 加载失败时的可见错误状态

页面打开时加载一次，之后每 5 秒刷新一次；页面销毁时清理定时器。刷新失败不清空上一次成功数据显示。

## 管理路由合同

注册 `GET api/usage`，不接受查询参数和正文，响应类型为 `application/json`。响应失败沿用现有管理接口错误格式 `{ "error": { "code", "message" } }`。

## 测试

- Rust 单元测试：初始快照为零；成功、失败和最近时间更新；并发更新不丢计数；管理路由返回字段和注册合同正确。
- 中间件测试：命中 BPS 后按 2xx/非 2xx/403/传输错误分类，未命中请求不增加计数。
- Worker 测试保持现有全量通过。
- 清单测试：资源 MIME、管理页和 `api/usage` 路由声明正确。
- 打包检查：三个 web 资源进入独立插件归档，归档无敏感文件。

## 边界

本版本不统计输入输出 Token、请求成本、按模型明细或跨重启历史。后续若需要持久化统计，可在不改变页面 JSON 合同的前提下增加按批次写入 RS state 的聚合快照。
