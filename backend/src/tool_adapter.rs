use crate::basispoints::normalize_request;
use crate::relay::{RelayContext, RelayError, ToolRelay};
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

pub(crate) struct PreparedRequest {
    pub(crate) request: Value,
    pub(crate) context: Option<RelayContext>,
}

pub(crate) fn prepare(relay: &ToolRelay, request: Value) -> Result<PreparedRequest, RelayError> {
    let mut request = normalize_request(request).map_err(|error| invalid(&error.to_string()))?;
    let source = request.as_object_mut().expect("normalized object");
    if source
        .get("previous_response_id")
        .is_some_and(|v| !v.is_null())
    {
        return Err(invalid(
            "Send the complete input history; previous_response_id is not supported",
        ));
    }
    let mut names = BTreeSet::new();
    if let Some(tools) = source.get("tools").filter(|v| !v.is_null()) {
        validate_tools(tools, "", 0, &mut names)?;
    }
    if let Some(items) = source.get("input").and_then(Value::as_array) {
        for item in items {
            if item.get("type").and_then(Value::as_str) == Some("additional_tools") {
                validate_tools(
                    item.get("tools")
                        .ok_or_else(|| invalid("additional_tools requires a tools array"))?,
                    "",
                    0,
                    &mut names,
                )?;
            }
        }
    }
    let wants_relay = !names.is_empty()
        || source.get("tool_choice").is_some_and(|v| !v.is_null())
        || source
            .get("parallel_tool_calls")
            .is_some_and(|v| !v.is_null())
        || source
            .get("input")
            .and_then(Value::as_array)
            .is_some_and(|items| {
                items.iter().any(|item| {
                    matches!(
                        item.get("type").and_then(Value::as_str),
                        Some(
                            "function_call"
                                | "custom_tool_call"
                                | "function_call_output"
                                | "custom_tool_call_output"
                                | "additional_tools"
                        )
                    )
                })
            });
    let context = if wants_relay {
        let prepared = relay.prepare_source(&*source)?;
        let mut input = Vec::new();
        if let Some(instructions) = source
            .remove("instructions")
            .and_then(|v| v.as_str().map(str::to_owned))
        {
            input.push(developer_message(&instructions));
        }
        if let Some(instructions) = prepared.developer_instructions {
            input.push(developer_message(&instructions));
        }
        input.extend(prepared.input);
        source.insert("input".to_owned(), Value::Array(input));
        Some(prepared.context)
    } else {
        None
    };
    for key in ["tools", "tool_choice", "parallel_tool_calls"] {
        source.remove(key);
    }
    Ok(PreparedRequest { request, context })
}

fn developer_message(text: &str) -> Value {
    json!({"type":"message","role":"developer","content":[{"type":"input_text","text":text}]})
}

fn invalid(message: &str) -> RelayError {
    RelayError {
        status: 400,
        code: "invalid_request",
        message: message.to_owned(),
    }
}

fn validate_tools(
    value: &Value,
    namespace: &str,
    depth: usize,
    names: &mut BTreeSet<String>,
) -> Result<(), RelayError> {
    if depth > 8 {
        return Err(invalid("Tool namespaces exceed the nesting limit"));
    }
    let tools = value
        .as_array()
        .ok_or_else(|| invalid("tools must be an array"))?;
    if tools.len() > 128 {
        return Err(invalid("Too many tools"));
    }
    for tool in tools {
        let object: &Map<String, Value> = tool
            .as_object()
            .ok_or_else(|| invalid("Tool declaration must be an object"))?;
        let name = object
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty() && v.len() <= 128)
            .ok_or_else(|| invalid("Tool name must contain 1 to 128 characters"))?;
        let extra = object
            .get("namespace")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim();
        let parent = [namespace, extra]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(".");
        let qualified = if parent.is_empty() {
            name.to_owned()
        } else {
            format!("{parent}.{name}")
        };
        if matches!(name, "run_officejs" | "functions.run_officejs")
            || matches!(
                qualified.as_str(),
                "run_officejs" | "functions.run_officejs"
            )
        {
            return Err(invalid("run_officejs is reserved for the BPS transport"));
        }
        match object.get("type").and_then(Value::as_str) {
            Some("namespace") => validate_tools(
                object
                    .get("tools")
                    .ok_or_else(|| invalid("Namespace requires tools"))?,
                &qualified,
                depth + 1,
                names,
            )?,
            Some("function" | "custom") => {
                if !names.insert(qualified) {
                    return Err(invalid("Duplicate tool name"));
                }
                if names.len() > 128 {
                    return Err(invalid("Too many tools"));
                }
            }
            _ => {
                return Err(invalid(
                    "Only client function, custom, and namespace tools are supported",
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn source() -> Value {
        json!({"model":"gpt-6-sol","input":"Weather in Shanghai?","tools":[{
            "type":"function","name":"get_weather","parameters":{
                "type":"object","properties":{"city":{"type":"string"}},"required":["city"],
                "additionalProperties":false
            }
        }],"tool_choice":"required","parallel_tool_calls":false})
    }

    #[test]
    fn translates_tool_catalog_to_developer_message_without_wire_tool_fields() {
        let prepared = prepare(&ToolRelay::new(), source()).unwrap();
        assert!(prepared.context.is_some());
        assert!(prepared.request.get("tools").is_none());
        assert!(prepared.request.get("tool_choice").is_none());
        assert!(prepared.request.get("parallel_tool_calls").is_none());
        assert_eq!(prepared.request["input"][0]["role"], "developer");
        assert!(
            prepared.request["input"][0]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("get_weather")
        );
    }

    #[test]
    fn rewrites_history_calls_and_results_even_without_current_tools() {
        let prepared = prepare(&ToolRelay::new(), json!({"model":"gpt-6-sol","input":[
            {"type":"function_call","call_id":"call_1","name":"get_weather","arguments":"{\"city\":\"Shanghai\"}"},
            {"type":"function_call_output","call_id":"call_1","output":"sunny"}
        ]})).unwrap();
        assert!(prepared.context.is_some());
        assert_eq!(prepared.request["input"][1]["name"], "run_officejs");
        assert_eq!(prepared.request["input"][2]["call_id"], "call_1");
    }

    #[test]
    fn rejects_reserved_transport_tools_and_unsupported_builtin_tools() {
        for name in ["run_officejs", "functions.run_officejs"] {
            let mut request = source();
            request["tools"][0]["name"] = json!(name);
            assert!(prepare(&ToolRelay::new(), request).is_err());
        }
        let request = json!({"model":"gpt-6-sol","input":"Search","tools":[{"type":"web_search"}]});
        assert!(prepare(&ToolRelay::new(), request).is_err());
    }

    #[test]
    fn rejects_response_id_continuation_and_keeps_regular_history() {
        assert!(
            prepare(
                &ToolRelay::new(),
                json!({"input":"hi","previous_response_id":"resp_1"})
            )
            .is_err()
        );
        let prepared=prepare(&ToolRelay::new(),json!({"model":"gpt-6-sol","input":[{"role":"user","content":"hi"},{"role":"assistant","content":"hello"}]})).unwrap();
        assert!(prepared.context.is_none());
        assert_eq!(prepared.request["input"].as_array().unwrap().len(), 2);
    }

    fn native_response(name: &str, payload: &str) -> Value {
        json!({"id":"resp_test","status":"completed","output":[{
            "type":"function_call","id":"fc_original","call_id":"call_test",
            "name":"run_officejs","status":"completed","arguments":json!({
                "summary":"Relay a client tool","extended_summary":"Protocol test",
                "destructive":false,"references":[name],"code":payload
            }).to_string()
        }]})
    }

    #[test]
    fn completed_function_response_replays_after_a_fresh_relay() {
        let prepared = prepare(&ToolRelay::new(), source()).unwrap();
        let native = native_response("get_weather", r#"{"city":"Shanghai"}"#);
        let (_, body) = crate::tool_response::transform(
            prepared.context.as_ref().unwrap(),
            false,
            "application/json",
            &serde_json::to_vec(&native).unwrap(),
        )
        .unwrap();
        let client: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(client["output"][0]["name"], "get_weather");
        assert_eq!(client["output"][0]["call_id"], "call_test");
        let mut history = client["output"].as_array().unwrap().clone();
        history.push(
            json!({"type":"function_call_output","call_id":"call_test","output":"fixture_sunny"}),
        );
        let replay = prepare(
            &ToolRelay::new(),
            json!({"model":"gpt-6-sol","input":history}),
        )
        .unwrap();
        let items = replay.request["input"].as_array().unwrap();
        let call = items
            .iter()
            .find(|item| item["type"] == "function_call")
            .unwrap();
        let outer: Value = serde_json::from_str(call["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(call["name"], "run_officejs");
        assert_eq!(outer["references"], json!(["get_weather"]));
        assert_eq!(outer["code"], r#"{"city":"Shanghai"}"#);
        assert!(
            items.iter().any(
                |item| item["type"] == "function_call_output" && item["call_id"] == "call_test"
            )
        );
    }

    #[test]
    fn custom_namespace_preserves_raw_payload_across_stream_and_replay() {
        let raw = "first \"quote\" \\ second\nline two\n";
        let prepared = prepare(
            &ToolRelay::new(),
            json!({"input":"write fixture","tools":[{
            "type":"namespace","name":"fixture","tools":[{"type":"custom","name":"write"}]
        }],"tool_choice":{"type":"custom","name":"write","namespace":"fixture"}}),
        )
        .unwrap();
        let (_, body) = crate::tool_response::transform(
            prepared.context.as_ref().unwrap(),
            true,
            "application/json",
            &serde_json::to_vec(&native_response("fixture.write", raw)).unwrap(),
        )
        .unwrap();
        let events = crate::sse::try_parse_sse_events(&body).unwrap();
        let terminal = events
            .iter()
            .find_map(|event| {
                let value: Value = serde_json::from_str(&event.data).ok()?;
                (value["type"] == "response.completed").then_some(value)
            })
            .unwrap();
        let call = terminal["response"]["output"][0].clone();
        assert_eq!(call["type"], "custom_tool_call");
        assert_eq!(call["namespace"], "fixture");
        assert_eq!(call["input"], raw);
        let replay = prepare(
            &ToolRelay::new(),
            json!({"input":[call,{
                "type":"custom_tool_call_output","call_id":"call_test","output":"fixture_written"
            }]}),
        )
        .unwrap();
        let call = replay.request["input"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "function_call")
            .unwrap();
        let outer: Value = serde_json::from_str(call["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(outer["code"], raw);
        assert_eq!(outer["references"], json!(["fixture.write"]));
    }

    #[test]
    fn choices_and_parallel_constraint_are_enforced_after_generation() {
        for choice in [json!("none"), json!({"type":"function","name":"other"})] {
            let mut request = source();
            request["tool_choice"] = choice;
            request["tools"]
                .as_array_mut()
                .unwrap()
                .push(json!({"type":"function","name":"other","parameters":{"type":"object"}}));
            let prepared = prepare(&ToolRelay::new(), request).unwrap();
            assert!(
                prepared
                    .context
                    .unwrap()
                    .transform_response(&native_response("get_weather", r#"{"city":"Shanghai"}"#))
                    .is_err()
            );
        }
        let prepared = prepare(&ToolRelay::new(), source()).unwrap();
        let mut response = native_response("get_weather", r#"{"city":"Shanghai"}"#);
        let mut second = response["output"][0].clone();
        second["call_id"] = json!("call_second");
        response["output"].as_array_mut().unwrap().push(second);
        assert!(
            prepared
                .context
                .unwrap()
                .transform_response(&response)
                .is_err()
        );
    }
}
