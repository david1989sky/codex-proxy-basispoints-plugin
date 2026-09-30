# Codex Proxy RS Basis Points 插件

这是一个独立的 Basis Points Responses companion 插件，插件 ID 为
`david1989sky.codex-proxy-basispoints`。它通过 RS 管理会话转发 Responses 请求并转换客户端工具调用，
并可在 request 中间件阶段接管配置模型的 Responses 请求。不包含账号管理或浏览器授权页面。

默认配置会接管 `gpt-6-sol`、`gpt-6-astra`、`gpt-5.6-sol` 和 `gpt-6-luna`，其他模型继续走 RS 原生 Provider。
可在插件配置中设置 `enabled` 和 `models`；`gpt-6.1-sol` 不在默认列表中，需先通过 BPS canary 确认可用后再加入。

## 功能

插件声明 RS `middleware` v3 的 `request` 阶段。被配置模型的普通 Responses 请求会由插件读取宿主 OAuth
凭据后直接发送到 BPS，并将 BPS JSON/SSE 响应返回给客户端；未命中模型列表的请求原样交给 RS 原生 Provider。
`GET api/status` 会报告 `middleware: true`，但只有命中模型列表的请求才会实际走 BPS。

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

插件通过 RS 的 `host.auth.get` 宿主回调读取请求的 OpenAI OAuth 账号，解析
`access_token` 和 JWT 的 `claims["https://api.openai.com/auth"].chatgpt_account_id`，再由
RS SDK 的 `host.http` 直接固定请求
[`https://bps.openai.com/basispoints/api/responses`](https://bps.openai.com/basispoints/api/responses)。
插件会为普通 Responses 请求补齐 Basis Points 所需的 `model_selection`、`stream`、`store`、
`reasoning_effort` 和会话 `metadata` 字段，并发送 Excel 客户端请求头。调用方不能提交 token、
目标 URL 或代理设置。普通请求保留上游 JSON/SSE 状态码和正文，正文上限为 2 MiB。access token 不会
经过 loopback Worker，也不会写入插件配置或日志。

## 能力边界

v0.1.7 支持字符串 `input`、标准 Responses message 数组、多轮上下文，以及客户端 `function`、
`custom` 和 `namespace` 工具。插件将工具目录和选择要求映射到 BPS 原生 `run_officejs` 协议，
再把返回值转换为标准 `function_call` / `custom_tool_call`。客户端负责执行工具并回传结果；
插件不会执行工具，也不会运行上游返回的 Office 或 JavaScript 代码。
中继响应的工具目录、选择方式、并行设置和 instructions 与调用方请求一致，包括 SSE 各响应快照。

每次请求都需携带完整历史，包括上一轮的完整 `response.output` 和对应工具结果。插件按请求
重建中继状态，不使用跨请求长期缓存；非 `null` 的 `previous_response_id` 会被拒绝。
支持 `tool_choice` 和 `parallel_tool_calls`，但不支持 `web_search`、`code_interpreter` 等内置工具。
`run_officejs` 是保留名称，不能作为客户端工具名。

函数参数校验只覆盖 JSON Schema 的部分规则，例如 `type`、`required`、`properties`、
`additionalProperties`、`items` 和 `enum`；不能视为完整的严格 JSON Schema 校验。
客户端应根据自身工具要求继续校验参数。

带工具中继的请求先缓冲完整上游响应（上限 2 MiB），校验后返回 JSON 或生成标准工具调用 SSE
事件。因此 `stream: true` 的工具请求不会实时返回首 token。普通请求保持原有 JSON/SSE 行为。

模型是否可用由 BPS 上游和宿主账号决定，插件不会把模型名限定为固定列表。
2026-09-30 已实测 `gpt-6-sol`、`gpt-5.6-sol`、`gpt-6-astra` 和 `gpt-6-luna` 可调用；
`gpt-6.1-sol` 当时返回 `403 basispoints_model_access_changed`。
宿主账号的模型目录不等于 BPS 可用模型列表。`model_not_found` 或模型权限错误由上游返回，
不应据此推断工具中继失败。

## 工具调用示例

向插件的 `api/basispoints/responses` 管理接口提交以下包装请求：

```json
{
  "accountId": "宿主账号 ID",
  "request": {
    "model": "gpt-6-sol",
    "input": [{"role": "user", "content": "上海天气如何？"}],
    "tools": [{
      "type": "function",
      "name": "get_weather",
      "description": "查询指定城市的天气",
      "parameters": {
        "type": "object",
        "properties": {"city": {"type": "string"}},
        "required": ["city"],
        "additionalProperties": false
      }
    }],
    "tool_choice": "required",
    "parallel_tool_calls": false,
    "stream": false
  }
}
```

取得标准 Responses 对象 `response` 后，客户端解析 `function_call.arguments` 并执行工具。
下一轮保留原始输入和完整输出，使用实际返回的 `call_id` 回传结果。例如，`body` 为上述请求：

```js
const call = response.output.find(item => item.type === "function_call");
const args = JSON.parse(call.arguments);
const result = await get_weather(args);
const nextBody = {
  accountId: body.accountId,
  request: {
    ...body.request,
    tool_choice: "auto",
    input: [
      ...body.request.input,
      ...response.output,
      {
        type: "function_call_output",
        call_id: call.call_id,
        output: JSON.stringify(result)
      }
    ]
  }
};
```

再次提交 `nextBody`，模型可结合工具结果继续回答。`custom_tool_call` 使用原始 `input` 字符串，
结果以 `custom_tool_call_output` 和相同 `call_id` 回传。多次工具调用需分别提供对应结果。

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

产物包含独立的 RS 插件归档、兼容性 Worker bundle、摘要清单和 SHA-256 文件。RS 插件本身不依赖
Worker。

## 安装

在 RS 的「插件管理」上传 BPS 插件归档，确认插件 ID 为
`david1989sky.codex-proxy-basispoints`。管理入口由 RS 管理员会话保护，账号凭据由宿主回调授权。
插件不读取浏览器 Cookie，不挂载凭据目录或密钥文件，也不需要额外的 Worker 地址配置。

## 第三方许可

工具中继和 SSE 基础实现改编自
[2han9wen71an/cpr-plugin-oai-basispoints](https://github.com/2han9wen71an/cpr-plugin-oai-basispoints)，
源提交为 `e23b69e99262eb50a424cfaed2b094d7f0fd900b`，采用 MIT 许可，原作者为 JaxsonWang。
完整声明见 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)；插件二进制的
`--third-party-notices` 参数可输出随包附带的声明。
