use crate::relay::{RelayContext, RelayError};
use crate::sse;
use serde_json::{Value, json};
use std::collections::BTreeSet;

enum Frame {
    Event { kind: String, value: Value },
    Opaque(Vec<u8>),
    Done(Vec<u8>),
}

struct ParsedStream {
    frames: Vec<Frame>,
    terminal_index: usize,
    response_id: Option<String>,
    native_events_seen: bool,
}

pub(crate) fn transform(
    context: &RelayContext,
    stream: bool,
    content_type: &str,
    body: &[u8],
) -> Result<(String, Vec<u8>), RelayError> {
    if let Ok(value) = serde_json::from_slice::<Value>(body) {
        validate_response(&value, stream)?;
        let response = complete_response(context, &value, None)?;
        return if stream {
            let mut output = Encoder::default();
            synthesize_response(&mut output, &response)?;
            Ok(("text/event-stream".to_owned(), output.bytes))
        } else {
            Ok(("application/json".to_owned(), encode_json(&response)?))
        };
    }
    let is_sse = content_type
        .split(';')
        .next()
        .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
        || std::str::from_utf8(body).is_ok_and(|text| {
            let text = text.trim_start_matches('\u{feff}').trim_start();
            ["event:", "data:", ":", "id:", "retry:"]
                .iter()
                .any(|prefix| text.starts_with(prefix))
        });
    if !is_sse {
        return Err(invalid("Basis Points returned invalid response JSON"));
    }
    let parsed = parse_stream(body)?;
    let Frame::Event { kind, value } = &parsed.frames[parsed.terminal_index] else {
        return Err(invalid(
            "Basis Points returned an invalid response terminal",
        ));
    };
    let mut response = terminal_response(value, kind, parsed.response_id.as_deref())?;
    let status = response_status(&response)?.to_owned();
    if stream && status != "completed" {
        let mut cleaned = response.clone();
        remove_tools(&mut cleaned)?;
        validate_response(&cleaned, true)?;
    }
    if stream && status == "completed" && has_tool_output(&response) {
        response_id(&response)?;
    }
    if status == "completed" && parsed.native_events_seen && !has_tool_output(&response) {
        return Err(invalid(
            "Basis Points returned tool events without a final client tool call",
        ));
    }
    response = complete_response(context, &response, Some(&status))?;
    if !stream {
        return Ok(("application/json".to_owned(), encode_json(&response)?));
    }
    let mut output = Encoder::default();
    if status != "completed" {
        synthesize_response(&mut output, &response)?;
        for frame in parsed.frames {
            if let Frame::Done(bytes) = frame {
                output.bytes.extend(bytes);
            }
        }
        return Ok(("text/event-stream".to_owned(), output.bytes));
    }
    let mut emitted_tools = BTreeSet::new();
    for (index, frame) in parsed.frames.into_iter().enumerate() {
        match frame {
            Frame::Opaque(bytes) | Frame::Done(bytes) => output.bytes.extend(bytes),
            Frame::Event { kind, mut value } => {
                if index == parsed.terminal_index {
                    if status == "completed" {
                        emit_tools(&mut output, &response, &mut emitted_tools, usize::MAX)?;
                    }
                    if value.get("response").is_some() || kind == "error" {
                        value["response"] = response.clone();
                    } else {
                        value = response.clone();
                        value["type"] = Value::String(kind.clone());
                    }
                    output.event(&kind, value)?;
                } else {
                    if status == "completed" {
                        let item_index = value
                            .get("output_index")
                            .and_then(Value::as_u64)
                            .and_then(|index| usize::try_from(index).ok())
                            .or_else(|| {
                                let call_id = value
                                    .pointer("/item/call_id")
                                    .or_else(|| value.get("call_id"))?
                                    .as_str()?;
                                response
                                    .get("output")?
                                    .as_array()?
                                    .iter()
                                    .position(|item| item["call_id"] == call_id)
                            });
                        if let Some(item_index) = item_index {
                            emit_tools(&mut output, &response, &mut emitted_tools, item_index)?;
                        }
                    }
                    if !is_native_event(&kind, &value) {
                        if let Some(snapshot) = value.get_mut("response") {
                            remove_tools(snapshot)?;
                            context.restore_metadata(snapshot);
                        }
                        output.event(&kind, value)?;
                    }
                }
            }
        }
    }
    Ok(("text/event-stream".to_owned(), output.bytes))
}

fn invalid(message: &'static str) -> RelayError {
    RelayError {
        status: 502,
        code: "invalid_upstream_response",
        message: message.to_owned(),
    }
}

fn encode_json(value: &Value) -> Result<Vec<u8>, RelayError> {
    serde_json::to_vec(value).map_err(|_| RelayError {
        status: 502,
        code: "relay_encoding_failed",
        message: "Client response could not be encoded".to_owned(),
    })
}

fn response_status(response: &Value) -> Result<&str, RelayError> {
    match response.get("status") {
        None => Ok("completed"),
        Some(Value::String(status))
            if matches!(
                status.as_str(),
                "completed" | "failed" | "incomplete" | "cancelled" | "canceled"
            ) =>
        {
            Ok(status)
        }
        _ => Err(invalid(
            "Basis Points returned an invalid final response status",
        )),
    }
}

fn complete_response(
    context: &RelayContext,
    response: &Value,
    status: Option<&str>,
) -> Result<Value, RelayError> {
    if !response.is_object() {
        return Err(invalid("Basis Points returned an invalid response object"));
    }
    let status = status.map_or_else(|| response_status(response), Ok)?;
    if status == "completed" {
        let mut response = context.transform_response(response)?.response;
        response["status"] = json!("completed");
        context.restore_metadata(&mut response);
        return Ok(response);
    }
    let mut response = response.clone();
    remove_tools(&mut response)?;
    context.restore_metadata(&mut response);
    Ok(response)
}

fn validate_response(response: &Value, synthesize: bool) -> Result<(), RelayError> {
    if !response.is_object() {
        return Err(invalid("Basis Points returned an invalid response object"));
    }
    let status = response_status(response)?;
    let items: &[Value] = match response.get("output") {
        Some(Value::Array(items)) => items,
        None if status != "completed" => &[],
        _ => {
            return Err(invalid(
                "Basis Points returned a response without an output array",
            ));
        }
    };
    if synthesize {
        response_id(response)?;
    }
    for item in items {
        if !item.is_object()
            || item
                .get("type")
                .and_then(Value::as_str)
                .is_none_or(str::is_empty)
        {
            return Err(invalid(
                "Basis Points returned an invalid response output item",
            ));
        }
        if !synthesize || is_tool_item(item) {
            continue;
        }
        if item
            .get("id")
            .and_then(Value::as_str)
            .is_none_or(str::is_empty)
        {
            return Err(invalid(
                "Basis Points returned an output item without an identifier",
            ));
        }
        if item["type"] != "message" {
            continue;
        }
        let content = item
            .get("content")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("Basis Points returned invalid message content"))?;
        for part in content {
            let kind = part
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("Basis Points returned invalid message content"))?;
            let field = match kind {
                "output_text" => "text",
                "refusal" => "refusal",
                _ => continue,
            };
            if !part.get(field).is_some_and(Value::is_string) {
                return Err(invalid("Basis Points returned invalid message content"));
            }
        }
    }
    Ok(())
}

fn remove_tools(response: &mut Value) -> Result<(), RelayError> {
    if !response.is_object() {
        return Err(invalid(
            "Basis Points returned an invalid response snapshot",
        ));
    }
    if let Some(output) = response.get_mut("output") {
        let output = output
            .as_array_mut()
            .ok_or_else(|| invalid("Basis Points response output must be an array"))?;
        output.retain(|item| !is_tool_item(item));
    } else {
        response["output"] = json!([]);
    }
    Ok(())
}

fn is_tool_item(item: &Value) -> bool {
    item.get("type")
        .and_then(Value::as_str)
        .is_some_and(|kind| matches!(kind, "function_call" | "custom_tool_call"))
}

fn has_tool_output(response: &Value) -> bool {
    response
        .get("output")
        .and_then(Value::as_array)
        .is_some_and(|output| output.iter().any(is_tool_item))
}

fn is_native_event(kind: &str, value: &Value) -> bool {
    match kind {
        "response.output_item.added" | "response.output_item.done" => {
            value.get("item").is_some_and(is_tool_item)
        }
        "response.function_call_arguments.delta"
        | "response.function_call_arguments.done"
        | "response.custom_tool_call_input.delta"
        | "response.custom_tool_call_input.done" => true,
        _ => false,
    }
}

// Validate the full buffered stream before the relay can update its native-call cache.
fn parse_stream(body: &[u8]) -> Result<ParsedStream, RelayError> {
    std::str::from_utf8(body).map_err(|_| invalid("Basis Points returned invalid UTF-8 SSE"))?;
    let mut splitter = sse::SseFrameSplitter::new();
    let wire_frames = splitter.feed(body);
    if splitter
        .finish()
        .is_some_and(|frame| frame.bytes.iter().any(|byte| !byte.is_ascii_whitespace()))
    {
        return Err(invalid("Basis Points returned an incomplete SSE frame"));
    }
    let mut frames = Vec::new();
    let mut terminal_index = None;
    let mut response_id = None;
    let mut native_events_seen = false;
    let mut done_seen = false;
    let mut sequence = None;
    for frame in wire_frames {
        let text = std::str::from_utf8(&frame.bytes)
            .map_err(|_| invalid("Basis Points returned invalid UTF-8 SSE"))?;
        let mut has_event_field = false;
        for line in text.trim_start_matches('\u{feff}').lines() {
            if line.is_empty() || line.starts_with(':') {
                continue;
            }
            let field = line.split_once(':').map_or(line, |(field, _)| field);
            if field == "event" {
                if has_event_field {
                    return Err(invalid("Basis Points returned duplicate SSE event fields"));
                }
                has_event_field = true;
            }
            if !matches!(field, "event" | "data" | "id" | "retry") {
                return Err(invalid("Basis Points returned a malformed SSE frame"));
            }
        }
        let events = sse::try_parse_sse_events(&frame.bytes)
            .map_err(|_| invalid("Basis Points returned invalid UTF-8 SSE"))?;
        if events.is_empty() {
            if has_event_field {
                return Err(invalid("Basis Points returned an SSE event without data"));
            }
            frames.push(Frame::Opaque(frame.bytes));
            continue;
        }
        if events.len() != 1 {
            return Err(invalid("Basis Points returned an invalid SSE frame"));
        }
        let event = &events[0];
        if event.is_done() {
            if terminal_index.is_none() || done_seen {
                return Err(invalid(
                    "Basis Points returned an invalid SSE done sentinel",
                ));
            }
            done_seen = true;
            frames.push(Frame::Done(frame.bytes));
            continue;
        }
        if terminal_index.is_some() || done_seen {
            return Err(invalid(
                "Basis Points returned events after the response terminal",
            ));
        }
        let event = sse::parse_json_event(event)
            .map_err(|_| invalid("Basis Points returned invalid SSE JSON"))?;
        if !event.data.is_object() {
            return Err(invalid("Basis Points returned an invalid SSE event object"));
        }
        let body_kind = match event.data.get("type") {
            Some(Value::String(kind)) if !kind.is_empty() => Some(kind.as_str()),
            None => None,
            _ => return Err(invalid("Basis Points returned an invalid SSE event type")),
        };
        if event
            .event
            .as_deref()
            .zip(body_kind)
            .is_some_and(|(wire, json)| wire != json)
        {
            return Err(invalid("Basis Points returned mismatched SSE event types"));
        }
        let kind = event
            .event
            .as_deref()
            .or(body_kind)
            .filter(|kind| !kind.is_empty())
            .ok_or_else(|| invalid("Basis Points returned an SSE event without a type"))?
            .to_owned();
        if kind.chars().any(char::is_control) {
            return Err(invalid("Basis Points returned an invalid SSE event type"));
        }
        if event
            .data
            .get("output_index")
            .is_some_and(|index| index.as_u64().is_none())
        {
            return Err(invalid("Basis Points returned an invalid SSE output index"));
        }
        if let Some(number) = event.data.get("sequence_number") {
            let number = number
                .as_u64()
                .ok_or_else(|| invalid("Basis Points returned an invalid SSE sequence number"))?;
            if sequence.is_some_and(|previous| number <= previous) {
                return Err(invalid(
                    "Basis Points returned unordered SSE sequence numbers",
                ));
            }
            sequence = Some(number);
        }
        for candidate in [
            event.data.pointer("/response/id"),
            event.data.get("response_id"),
            sse::is_terminal_event(&kind)
                .then(|| event.data.get("id"))
                .flatten(),
        ]
        .into_iter()
        .flatten()
        {
            let candidate = candidate
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| invalid("Basis Points returned an invalid response identifier"))?;
            if response_id.as_deref().is_some_and(|id| id != candidate) {
                return Err(invalid(
                    "Basis Points returned mismatched response identifiers",
                ));
            }
            response_id = Some(candidate.to_owned());
        }
        if let Some(snapshot) = event.data.get("response") {
            let mut snapshot = snapshot.clone();
            remove_tools(&mut snapshot)?;
            if !sse::is_terminal_event(&kind) {
                let expected = match kind.as_str() {
                    "response.created" | "response.in_progress" => Some("in_progress"),
                    "response.queued" => Some("queued"),
                    _ => None,
                };
                if let Some(expected) = expected
                    && let Some(status) = snapshot.get("status")
                    && status.as_str() != Some(expected)
                {
                    return Err(invalid(
                        "Basis Points returned mismatched event and response status",
                    ));
                }
            }
        }
        native_events_seen |= is_native_event(&kind, &event.data);
        if sse::is_terminal_event(&kind) {
            terminal_response(&event.data, &kind, response_id.as_deref())?;
            terminal_index = Some(frames.len());
        }
        frames.push(Frame::Event {
            kind,
            value: event.data,
        });
    }
    Ok(ParsedStream {
        frames,
        terminal_index: terminal_index.ok_or_else(|| {
            invalid("Basis Points response stream ended without a terminal event")
        })?,
        response_id,
        native_events_seen,
    })
}

fn terminal_response(
    value: &Value,
    kind: &str,
    response_id: Option<&str>,
) -> Result<Value, RelayError> {
    let mut response = if kind == "error" && value.get("response").is_none() {
        let mut error = value.get("error").unwrap_or(value).clone();
        if let Some(object) = error.as_object_mut() {
            object.remove("type");
            object.remove("sequence_number");
        }
        json!({"id":response_id.unwrap_or("resp_upstream_error"),"object":"response",
            "status":"failed","output":[],"error":error})
    } else {
        value.get("response").unwrap_or(value).clone()
    };
    let object = response
        .as_object_mut()
        .ok_or_else(|| invalid("Basis Points returned an invalid response terminal"))?;
    let status = match object.get("status") {
        Some(Value::String(status)) => status.as_str(),
        None => match kind {
            "response.incomplete" => "incomplete",
            "response.failed" | "error" => "failed",
            "response.cancelled" => "cancelled",
            "response.canceled" => "canceled",
            _ => "completed",
        },
        _ => return Err(invalid("Basis Points returned an invalid terminal status")),
    };
    let valid = match kind {
        "response.completed" => status == "completed",
        "response.incomplete" => status == "incomplete",
        "response.failed" | "error" => status == "failed",
        "response.cancelled" | "response.canceled" => matches!(status, "cancelled" | "canceled"),
        "response.done" => matches!(
            status,
            "completed" | "incomplete" | "failed" | "cancelled" | "canceled"
        ),
        _ => false,
    };
    if !valid {
        return Err(invalid(
            "Basis Points returned mismatched terminal event and status",
        ));
    }
    let status = status.to_owned();
    object.insert("status".to_owned(), Value::String(status));
    if value.get("response").is_none() {
        object.remove("type");
        object.remove("sequence_number");
    }
    if !object.contains_key("id")
        && let Some(id) = response_id
    {
        object.insert("id".to_owned(), Value::String(id.to_owned()));
    }
    validate_response(&response, false)?;
    Ok(response)
}

#[derive(Default)]
struct Encoder {
    bytes: Vec<u8>,
    sequence: u64,
}

impl Encoder {
    fn event(&mut self, kind: &str, mut value: Value) -> Result<(), RelayError> {
        let object = value
            .as_object_mut()
            .ok_or_else(|| invalid("Client event is not an object"))?;
        object.insert("type".to_owned(), Value::String(kind.to_owned()));
        object.insert("sequence_number".to_owned(), Value::from(self.sequence));
        self.sequence = self
            .sequence
            .checked_add(1)
            .ok_or_else(|| invalid("Client event sequence overflowed"))?;
        self.bytes
            .extend_from_slice(format!("event: {kind}\ndata: ").as_bytes());
        self.bytes.extend(encode_json(&value)?);
        self.bytes.extend_from_slice(b"\n\n");
        Ok(())
    }
}

fn response_id(response: &Value) -> Result<&str, RelayError> {
    response
        .get("id")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .ok_or_else(|| invalid("Basis Points returned a response without an identifier"))
}

fn emit_tools(
    output: &mut Encoder,
    response: &Value,
    emitted: &mut BTreeSet<usize>,
    through: usize,
) -> Result<(), RelayError> {
    let Some(items) = response.get("output").and_then(Value::as_array) else {
        return Ok(());
    };
    for (index, item) in items
        .iter()
        .enumerate()
        .take_while(|(index, _)| *index <= through)
    {
        if is_tool_item(item) && emitted.insert(index) {
            emit_tool(output, response_id(response)?, index, item)?;
        }
    }
    Ok(())
}

fn emit_tool(
    output: &mut Encoder,
    response_id: &str,
    index: usize,
    item: &Value,
) -> Result<(), RelayError> {
    let custom = item["type"] == "custom_tool_call";
    let field = if custom { "input" } else { "arguments" };
    let prefix = if custom {
        "response.custom_tool_call_input"
    } else {
        "response.function_call_arguments"
    };
    let payload = item
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("Client tool payload is invalid"))?;
    let mut added = item.clone();
    added[field] = Value::String(String::new());
    added["status"] = json!("in_progress");
    output.event(
        "response.output_item.added",
        json!({"response_id":response_id,"output_index":index,"item":added}),
    )?;
    let mut arguments = json!({"response_id":response_id,"output_index":index,"item_id":item["id"],"call_id":item["call_id"]});
    arguments["delta"] = json!(payload);
    output.event(&format!("{prefix}.delta"), arguments.clone())?;
    arguments
        .as_object_mut()
        .expect("arguments event object")
        .remove("delta");
    arguments[field] = json!(payload);
    output.event(&format!("{prefix}.done"), arguments)?;
    output.event(
        "response.output_item.done",
        json!({"response_id":response_id,"output_index":index,"item":item}),
    )
}

fn synthesize_response(output: &mut Encoder, response: &Value) -> Result<(), RelayError> {
    let id = response_id(response)?;
    let mut created = response.clone();
    created["status"] = json!("in_progress");
    created["output"] = json!([]);
    output.event("response.created", json!({"response":created}))?;
    if let Some(items) = response.get("output").and_then(Value::as_array) {
        for (index, item) in items.iter().enumerate() {
            if is_tool_item(item) {
                emit_tool(output, id, index, item)?;
            } else {
                emit_item(output, id, index, item)?;
            }
        }
    }
    let kind = format!("response.{}", response_status(response)?);
    output.event(&kind, json!({"response":response}))
}

fn emit_item(
    output: &mut Encoder,
    response_id: &str,
    index: usize,
    item: &Value,
) -> Result<(), RelayError> {
    let mut added = item.clone();
    if item.get("type").and_then(Value::as_str) == Some("message") {
        added["content"] = json!([]);
        added["status"] = json!("in_progress");
    }
    output.event(
        "response.output_item.added",
        json!({"response_id":response_id,"output_index":index,"item":added}),
    )?;
    if let Some(content) = item.get("content").and_then(Value::as_array) {
        for (content_index, part) in content.iter().enumerate() {
            let (field, prefix) = match part.get("type").and_then(Value::as_str) {
                Some("output_text") => ("text", "response.output_text"),
                Some("refusal") => ("refusal", "response.refusal"),
                _ => continue,
            };
            let text = part
                .get(field)
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("Basis Points returned invalid message content"))?;
            let mut event = json!({"response_id":response_id,"output_index":index,"item_id":item["id"],"content_index":content_index});
            let mut added = part.clone();
            added[field] = json!("");
            event["part"] = added;
            output.event("response.content_part.added", event.clone())?;
            event
                .as_object_mut()
                .expect("content event object")
                .remove("part");
            event["delta"] = json!(text);
            output.event(&format!("{prefix}.delta"), event.clone())?;
            event
                .as_object_mut()
                .expect("content event object")
                .remove("delta");
            event[field] = json!(text);
            output.event(&format!("{prefix}.done"), event.clone())?;
            event
                .as_object_mut()
                .expect("content event object")
                .remove(field);
            event["part"] = part.clone();
            output.event("response.content_part.done", event)?;
        }
    }
    output.event(
        "response.output_item.done",
        json!({"response_id":response_id,"output_index":index,"item":item}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{relay::ToolRelay, sse};
    use serde_json::{Value, json};

    fn context(relay: &ToolRelay, required: bool) -> RelayContext {
        relay
            .prepare_source(&json!({
                "tools": [
                    {"type":"function","name":"exec","parameters":{
                        "type":"object","required":["x"],"properties":{"x":{"type":"integer"}},
                        "additionalProperties":false
                    }},
                    {"type":"custom","name":"patch"}
                ],
                "tool_choice": if required { "required" } else { "auto" }
            }))
            .unwrap()
            .context
    }

    fn native(tool: &str, code: &str) -> Value {
        json!({"type":"function_call","id":"fc_native","call_id":"call_1",
            "name":"run_officejs","arguments":json!({"references":[tool],"code":code}).to_string()})
    }

    fn message() -> Value {
        json!({"type":"message","id":"msg_1","role":"assistant","status":"completed",
            "content":[{"type":"output_text","text":"Working","annotations":[]}]})
    }

    fn response(status: &str, tool: &str, code: &str) -> Value {
        json!({"id":"resp_1","object":"response","status":status,
            "output":[message(),native(tool,code)]})
    }

    fn frame(kind: &str, value: Value) -> String {
        format!("event: {kind}\ndata: {value}\n\n")
    }

    fn stream_body(status: &str, tool: &str, code: &str, done: bool) -> String {
        let item = native(tool, code);
        let terminal_kind = format!("response.{status}");
        let mut body = frame(
            "response.created",
            json!({"type":"response.created","sequence_number":0,
            "response":{"id":"resp_1","status":"in_progress","output":[item]}}),
        );
        body.push_str(&frame(
            "response.output_text.delta",
            json!({"type":"response.output_text.delta",
            "sequence_number":1,"response_id":"resp_1","item_id":"msg_1","output_index":0,
            "content_index":0,"delta":"Working"}),
        ));
        body.push_str(&frame(
            "response.output_item.added",
            json!({"type":"response.output_item.added",
            "sequence_number":2,"response_id":"resp_1","output_index":1,"item":native(tool,code)}),
        ));
        body.push_str(&frame("response.function_call_arguments.delta", json!({
            "type":"response.function_call_arguments.delta","sequence_number":3,
            "response_id":"resp_1","item_id":"fc_native","output_index":1,"delta":"partial-native"})));
        body.push_str(&frame(
            "response.output_item.done",
            json!({"type":"response.output_item.done",
            "sequence_number":4,"response_id":"resp_1","output_index":1,"item":native(tool,code)}),
        ));
        body.push_str(&frame(
            &terminal_kind,
            json!({"type":terminal_kind,"sequence_number":9,
            "response":response(status,tool,code)}),
        ));
        if done {
            body.push_str("data: [DONE]\n\n");
        }
        body
    }

    fn events(bytes: &[u8]) -> Vec<Value> {
        sse::parse_sse_json_events(bytes)
            .unwrap()
            .into_iter()
            .map(|event| event.data)
            .collect()
    }

    fn assert_sequences(events: &[Value]) {
        for (index, event) in events.iter().enumerate() {
            assert_eq!(event["sequence_number"].as_u64(), Some(index as u64));
        }
    }

    fn upstream_metadata(mut response: Value) -> Value {
        response["tools"] = json!([{"type":"function","name":"run_officejs"}]);
        response["instructions"] = json!("Internal Office instructions");
        response["tool_choice"] = json!("auto");
        response["parallel_tool_calls"] = json!(true);
        response["tool_usage"] =
            json!({"image_gen":{"total_tokens":0},"web_search":{"num_requests":0}});
        response
    }

    #[test]
    fn relay_json_restores_original_caller_metadata_and_additional_tools() {
        let relay = ToolRelay::new();
        let source = json!({
            "tools":[{"type":"namespace","name":"utils","tools":[
                {"type":"function","name":"exec","parameters":{"type":"object"}}
            ]}],
            "input":[{"type":"additional_tools","tools":[{"type":"custom","name":"patch"}]}],
            "instructions":"Caller instructions",
            "tool_choice":{"type":"function","name":"exec","namespace":"utils"},
            "parallel_tool_calls":false
        });
        let context = relay.prepare_source(&source).unwrap().context;
        let body = upstream_metadata(response("completed", "utils.exec", "{}"));
        let (_, bytes) = transform(
            &context,
            false,
            "application/json",
            body.to_string().as_bytes(),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(
            value["tools"],
            json!([source["tools"][0], source["input"][0]["tools"][0]])
        );
        assert_eq!(value["instructions"], source["instructions"]);
        assert_eq!(value["tool_choice"], source["tool_choice"]);
        assert_eq!(value["parallel_tool_calls"], false);
        assert_eq!(value["tool_usage"], body["tool_usage"]);
        assert!(!String::from_utf8(bytes).unwrap().contains("run_officejs"));
    }

    #[test]
    fn relay_sse_restores_metadata_on_every_response_snapshot() {
        let relay = ToolRelay::new();
        let source = json!({"tools":[{"type":"custom","name":"patch"}],
            "instructions":"Caller instructions","tool_choice":{"type":"custom","name":"patch"},
            "parallel_tool_calls":false});
        let context = relay.prepare_source(&source).unwrap().context;
        for status in ["completed", "failed"] {
            let mut body = String::new();
            for kind in ["response.created", "response.in_progress"] {
                body.push_str(&frame(
                    kind,
                    json!({"type":kind,"response":upstream_metadata(json!({
                    "id":"resp_1","status":"in_progress","output":[]}))}),
                ));
            }
            let terminal = format!("response.{status}");
            body.push_str(&frame(
                &terminal,
                json!({"type":terminal,"response":upstream_metadata(
                response(status,"patch","raw custom input"))}),
            ));
            let (_, bytes) =
                transform(&context, true, "text/event-stream", body.as_bytes()).unwrap();
            for event in events(&bytes) {
                if let Some(snapshot) = event.get("response") {
                    assert_eq!(snapshot["tools"], source["tools"]);
                    assert_eq!(snapshot["instructions"], source["instructions"]);
                    assert_eq!(snapshot["tool_choice"], source["tool_choice"]);
                    assert_eq!(snapshot["parallel_tool_calls"], false);
                    assert_eq!(
                        snapshot["tool_usage"],
                        json!({"image_gen":{"total_tokens":0},"web_search":{"num_requests":0}})
                    );
                }
            }
        }
    }

    #[test]
    fn history_without_current_catalog_restores_empty_tools_and_metadata_defaults() {
        let relay = ToolRelay::new();
        let context = relay
            .prepare_source(&json!({"input":[
                {"type":"function_call","name":"exec","call_id":"call_1","arguments":"{}"},
                {"type":"function_call_output","call_id":"call_1","output":"ok"}
            ]}))
            .unwrap()
            .context;
        assert!(context.is_active());
        for status in ["completed", "failed"] {
            let body =
                upstream_metadata(json!({"id":"resp_1","status":status,"output":[message()]}));
            let (_, bytes) = transform(
                &context,
                false,
                "application/json",
                body.to_string().as_bytes(),
            )
            .unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["tools"], json!([]));
            assert_eq!(value["instructions"], Value::Null);
            assert_eq!(value["tool_choice"], "auto");
            assert_eq!(value["parallel_tool_calls"], true);
        }
    }

    fn assert_sdk_output(events: &[Value]) {
        let mut output = Vec::<Value>::new();
        for event in events {
            let kind = event["type"].as_str().unwrap();
            if kind == "response.created" {
                continue;
            }
            if sse::is_terminal_event(kind) {
                assert_eq!(json!(output), event["response"]["output"]);
                continue;
            }
            let index = event["output_index"].as_u64().unwrap() as usize;
            if kind == "response.output_item.added" {
                assert_eq!(index, output.len(), "Item added out of order: {event}");
                output.push(event["item"].clone());
                continue;
            }
            let item = output
                .get_mut(index)
                .expect("delta/done routes an added item");
            if kind == "response.output_item.done" {
                assert_eq!(item["id"], event["item"]["id"]);
                for field in ["content", "arguments", "input", "call_id"] {
                    if item.get(field).is_some() {
                        assert_eq!(item[field], event["item"][field]);
                    }
                }
                *item = event["item"].clone();
                continue;
            }
            assert_eq!(item["id"], event["item_id"]);
            if kind.starts_with("response.function_call_arguments.")
                || kind.starts_with("response.custom_tool_call_input.")
            {
                assert_eq!(item["call_id"], event["call_id"]);
                let field = if kind.starts_with("response.function_call_arguments.") {
                    "arguments"
                } else {
                    "input"
                };
                if kind.ends_with(".delta") {
                    let text = format!(
                        "{}{}",
                        item[field].as_str().unwrap(),
                        event["delta"].as_str().unwrap()
                    );
                    item[field] = json!(text);
                } else {
                    assert_eq!(item[field], event[field]);
                }
                continue;
            }
            let part_index = event["content_index"].as_u64().unwrap() as usize;
            let content = item["content"].as_array_mut().unwrap();
            if kind == "response.content_part.added" {
                assert_eq!(part_index, content.len());
                content.push(event["part"].clone());
            } else {
                let part = content
                    .get_mut(part_index)
                    .expect("delta/done routes an added content part");
                let field = if kind.starts_with("response.refusal.") {
                    "refusal"
                } else {
                    "text"
                };
                if kind.ends_with(".delta") {
                    let text = format!(
                        "{}{}",
                        part[field].as_str().unwrap(),
                        event["delta"].as_str().unwrap()
                    );
                    part[field] = json!(text);
                } else if kind == "response.content_part.done" {
                    assert_eq!(*part, event["part"]);
                } else {
                    assert_eq!(part[field], event[field]);
                }
            }
        }
    }

    #[test]
    fn json_function_response_preserves_text_and_validates_client_arguments() {
        let relay = ToolRelay::new();
        let body = response("completed", "exec", r#"{"x":9007199254740993}"#).to_string();
        let (content_type, bytes) = transform(
            &context(&relay, true),
            false,
            "application/json",
            body.as_bytes(),
        )
        .unwrap();
        assert_eq!(content_type, "application/json");
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["output"][0], message());
        assert_eq!(value["output"][1]["name"], "exec");
        assert_eq!(value["output"][1]["call_id"], "call_1");
        assert_eq!(value["output"][1]["arguments"], r#"{"x":9007199254740993}"#);
        assert!(!String::from_utf8(bytes).unwrap().contains("run_officejs"));
        assert_eq!(relay.cache_len(), 1);
    }

    #[test]
    fn streamed_function_emits_complete_client_lifecycle_with_stable_identifiers() {
        let relay = ToolRelay::new();
        let body = stream_body("completed", "exec", r#"{"x":1}"#, true);
        let (content_type, bytes) = transform(
            &context(&relay, true),
            true,
            "text/event-stream; charset=utf-8",
            body.as_bytes(),
        )
        .unwrap();
        assert_eq!(content_type, "text/event-stream");
        let wire = String::from_utf8(bytes.clone()).unwrap();
        assert!(wire.ends_with("data: [DONE]\n\n"));
        assert!(!wire.contains("run_officejs"));
        assert!(!wire.contains("partial-native"));
        let events = events(&bytes);
        assert_eq!(events.len(), 7);
        assert_sequences(&events);
        assert_eq!(events[0]["response"]["output"], json!([]));
        assert_eq!(events[1]["delta"], "Working");
        assert_eq!(events[2]["type"], "response.output_item.added");
        assert_eq!(events[2]["item"]["name"], "exec");
        assert_eq!(events[2]["item"]["arguments"], "");
        assert_eq!(events[2]["item"]["status"], "in_progress");
        assert_eq!(events[3]["type"], "response.function_call_arguments.delta");
        assert_eq!(events[3]["delta"], r#"{"x":1}"#);
        assert_eq!(events[4]["arguments"], r#"{"x":1}"#);
        assert_eq!(events[5]["type"], "response.output_item.done");
        for event in &events[2..6] {
            assert_eq!(event["response_id"], "resp_1");
            assert_eq!(event["output_index"], 1);
        }
        for event in &events[3..5] {
            assert_eq!(event["item_id"], "fc_native");
            assert_eq!(event["call_id"], "call_1");
        }
        assert_eq!(events[6]["response"]["output"][1]["name"], "exec");
    }

    #[test]
    fn custom_lifecycle_preserves_raw_input_and_custom_item_id() {
        let relay = ToolRelay::new();
        let raw = "*** Begin Patch\n*** End Patch\n";
        let body = stream_body("completed", "patch", raw, false);
        let (_, bytes) = transform(
            &context(&relay, true),
            true,
            "text/event-stream",
            body.as_bytes(),
        )
        .unwrap();
        let events = events(&bytes);
        assert_eq!(events[2]["item"]["type"], "custom_tool_call");
        assert_eq!(events[2]["item"]["id"], "ctc_native");
        assert_eq!(events[2]["item"]["input"], "");
        assert_eq!(events[3]["type"], "response.custom_tool_call_input.delta");
        assert_eq!(events[3]["item_id"], "ctc_native");
        assert_eq!(events[3]["delta"], raw);
        assert_eq!(events[4]["type"], "response.custom_tool_call_input.done");
        assert_eq!(events[4]["input"], raw);
        assert_sequences(&events);
    }

    #[test]
    fn non_stream_sse_extracts_terminal_json_without_requiring_done() {
        let relay = ToolRelay::new();
        for done in [false, true] {
            let body = stream_body("completed", "exec", r#"{"x":1}"#, done);
            let (content_type, bytes) = transform(
                &context(&relay, true),
                false,
                "text/event-stream",
                body.as_bytes(),
            )
            .unwrap();
            assert_eq!(content_type, "application/json");
            let response: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(response["id"], "resp_1");
            assert_eq!(response["output"][1]["name"], "exec");
        }
    }

    #[test]
    fn json_stream_is_synthesized_with_text_and_tool_events() {
        let relay = ToolRelay::new();
        let body = response("completed", "exec", r#"{"x":1}"#).to_string();
        let (content_type, bytes) = transform(
            &context(&relay, true),
            true,
            "application/json",
            body.as_bytes(),
        )
        .unwrap();
        assert_eq!(content_type, "text/event-stream");
        let events = events(&bytes);
        assert_eq!(events.first().unwrap()["type"], "response.created");
        assert_eq!(events.first().unwrap()["response"]["status"], "in_progress");
        assert_eq!(events.first().unwrap()["response"]["output"], json!([]));
        assert!(
            events
                .iter()
                .any(|event| event["type"] == "response.output_text.delta"
                    && event["delta"] == "Working")
        );
        assert!(events.iter().any(|event| event["type"]
            == "response.function_call_arguments.delta"
            && event["delta"] == r#"{"x":1}"#));
        assert_eq!(events.last().unwrap()["type"], "response.completed");
        assert_sequences(&events);
        assert!(!String::from_utf8(bytes).unwrap().contains("run_officejs"));
    }

    #[test]
    fn failed_and_incomplete_streams_remove_partial_tools_without_cache_updates() {
        for status in ["failed", "incomplete"] {
            let relay = ToolRelay::new();
            let body = stream_body(status, "exec", "invalid partial code", true);
            for stream in [false, true] {
                let (_, bytes) = transform(
                    &context(&relay, true),
                    stream,
                    "text/event-stream",
                    body.as_bytes(),
                )
                .unwrap();
                assert!(
                    !String::from_utf8(bytes.clone())
                        .unwrap()
                        .contains("run_officejs")
                );
                let terminal = if stream {
                    events(&bytes).last().unwrap()["response"].clone()
                } else {
                    serde_json::from_slice(&bytes).unwrap()
                };
                assert_eq!(terminal["status"], status);
                assert_eq!(terminal["output"], json!([message()]));
                if stream {
                    assert!(
                        !events(&bytes)
                            .iter()
                            .any(|event| is_tool_item(&event["item"]))
                    );
                }
                assert_eq!(relay.cache_len(), 0);
            }
        }
    }

    #[test]
    fn failed_json_preserves_error_and_bypasses_required_tool_choice() {
        let relay = ToolRelay::new();
        let mut body = response("failed", "exec", "partial");
        body["error"] = json!({"code":"upstream_failure","message":"Upstream request failed"});
        let (_, bytes) = transform(
            &context(&relay, true),
            false,
            "application/json",
            body.to_string().as_bytes(),
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["error"], body["error"]);
        assert_eq!(value["output"], json!([message()]));
        assert_eq!(relay.cache_len(), 0);
    }

    #[test]
    fn malformed_or_missing_terminal_is_rejected() {
        let relay = ToolRelay::new();
        for body in [
            "event: response.output_text.delta\ndata: {\n\n",
            "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"a\"}\n\n",
            "data: [DONE]\n\n",
            "event: response.completed\n\n",
        ] {
            assert!(
                transform(
                    &context(&relay, false),
                    true,
                    "text/event-stream",
                    body.as_bytes()
                )
                .is_err(),
                "{body}"
            );
        }
    }

    #[test]
    fn mismatched_event_types_terminal_status_and_response_ids_are_rejected() {
        let relay = ToolRelay::new();
        let cases = [
            frame(
                "response.completed",
                json!({"type":"response.failed","response":{"id":"resp_1","status":"completed","output":[]}}),
            ),
            frame(
                "response.completed",
                json!({"type":"response.completed","response":{"id":"resp_1","status":"failed","output":[]}}),
            ),
            format!(
                "{}{}",
                frame(
                    "response.created",
                    json!({"type":"response.created","response":{"id":"resp_1","status":"in_progress","output":[]}})
                ),
                frame(
                    "response.completed",
                    json!({"type":"response.completed","response":{"id":"resp_2","status":"completed","output":[]}})
                )
            ),
        ];
        for body in cases {
            assert!(
                transform(
                    &context(&relay, false),
                    true,
                    "text/event-stream",
                    body.as_bytes()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn validates_entire_stream_before_updating_cache() {
        let relay = ToolRelay::new();
        let body = format!(
            "{}data: malformed\n\n",
            stream_body("completed", "exec", r#"{"x":1}"#, false)
        );
        assert!(
            transform(
                &context(&relay, true),
                true,
                "text/event-stream",
                body.as_bytes()
            )
            .is_err()
        );
        assert_eq!(relay.cache_len(), 0);
    }

    #[test]
    fn completed_native_events_without_final_call_are_rejected() {
        let relay = ToolRelay::new();
        let body = format!(
            "{}{}",
            frame(
                "response.function_call_arguments.delta",
                json!({"type":"response.function_call_arguments.delta","delta":"partial"})
            ),
            frame(
                "response.completed",
                json!({"type":"response.completed","response":{"id":"resp_1","status":"completed","output":[]}})
            )
        );
        assert!(
            transform(
                &context(&relay, false),
                true,
                "text/event-stream",
                body.as_bytes()
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_unstreamable_json_before_updating_cache() {
        for missing_id in [false, true] {
            let relay = ToolRelay::new();
            let mut body = response("completed", "exec", r#"{"x":1}"#);
            if missing_id {
                body.as_object_mut().unwrap().remove("id");
            } else {
                body["output"][0]["content"][0]["text"] = json!(42);
            }
            assert!(
                transform(
                    &context(&relay, true),
                    true,
                    "application/json",
                    body.to_string().as_bytes()
                )
                .is_err()
            );
            assert_eq!(relay.cache_len(), 0);
        }
    }

    #[test]
    fn error_terminal_retains_upstream_error_in_non_stream_response() {
        let relay = ToolRelay::new();
        let error = json!({"type":"error","code":"upstream_failure","message":"Request failed"});
        let body = format!(
            "{}{}",
            frame(
                "response.created",
                json!({"type":"response.created","response":{"id":"resp_1","status":"in_progress","output":[]}})
            ),
            frame("error", error.clone())
        );
        let (_, bytes) = transform(
            &context(&relay, true),
            false,
            "text/event-stream",
            body.as_bytes(),
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(response["status"], "failed");
        assert_eq!(response["error"]["code"], error["code"]);
        assert_eq!(response["error"]["message"], error["message"]);
        assert_eq!(response["output"], json!([]));
        assert_eq!(relay.cache_len(), 0);
    }

    #[test]
    fn rejects_mismatched_created_status_and_missing_terminal_response() {
        let relay = ToolRelay::new();
        for body in [
            format!(
                "{}{}",
                frame(
                    "response.created",
                    json!({"type":"response.created","response":{"id":"resp_1","status":"completed","output":[]}})
                ),
                frame(
                    "response.completed",
                    json!({"type":"response.completed","response":{"id":"resp_1","status":"completed","output":[]}})
                )
            ),
            frame("response.completed", json!({"type":"response.completed"})),
            frame(
                "response.completed",
                json!({"type":"response.completed","response":{"id":"resp_1","status":"completed"}}),
            ),
        ] {
            assert!(
                transform(
                    &context(&relay, false),
                    true,
                    "text/event-stream",
                    body.as_bytes()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn crlf_data_only_events_and_comments_are_preserved_semantically() {
        let relay = ToolRelay::new();
        let body = format!(
            ": keepalive\r\n\r\ndata: {}\r\n\r\ndata: [DONE]\r\n\r\n",
            json!({"type":"response.completed","response":response("completed","exec",r#"{"x":1}"#)})
        );
        let (_, bytes) = transform(
            &context(&relay, true),
            true,
            "text/event-stream",
            body.as_bytes(),
        )
        .unwrap();
        let wire = String::from_utf8(bytes.clone()).unwrap();
        assert!(wire.starts_with(": keepalive\r\n\r\n"));
        assert!(wire.ends_with("data: [DONE]\r\n\r\n"));
        assert_sequences(&events(&bytes));
        assert_eq!(
            events(&bytes).last().unwrap()["response"]["output"][1]["name"],
            "exec"
        );
    }

    #[test]
    fn data_only_event_types_cannot_inject_sse_fields() {
        let relay = ToolRelay::new();
        let body = format!(
            "data: {}\n\n{}",
            json!({"type":"other\ndata: injected","sequence_number":0}),
            frame(
                "response.completed",
                json!({"type":"response.completed","response":{"id":"resp_1","status":"completed","output":[]}})
            )
        );
        assert!(
            transform(
                &context(&relay, false),
                true,
                "text/event-stream",
                body.as_bytes()
            )
            .is_err()
        );
    }

    #[test]
    fn named_done_sentinel_is_preserved() {
        let relay = ToolRelay::new();
        let body = format!(
            "{}event: done\ndata: [DONE]\n\n",
            stream_body("completed", "exec", r#"{"x":1}"#, false)
        );
        let (_, bytes) = transform(
            &context(&relay, true),
            true,
            "text/event-stream",
            body.as_bytes(),
        )
        .unwrap();
        assert!(
            String::from_utf8(bytes)
                .unwrap()
                .ends_with("event: done\ndata: [DONE]\n\n")
        );
    }

    #[test]
    fn failed_response_without_output_keeps_failure_without_tool_execution() {
        let relay = ToolRelay::new();
        let body = json!({"id":"resp_1","status":"failed","error":{"code":"upstream_failure"}});
        let (_, bytes) = transform(
            &context(&relay, true),
            false,
            "application/json",
            body.to_string().as_bytes(),
        )
        .unwrap();
        let response: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(response["status"], "failed");
        assert_eq!(response["error"], body["error"]);
        assert_eq!(response["output"], json!([]));
        assert_eq!(relay.cache_len(), 0);
    }

    #[test]
    fn native_first_output_keeps_tool_before_later_message_without_duplicate_lifecycle() {
        for include_native_added in [false, true] {
            let relay = ToolRelay::new();
            let mut terminal = response("completed", "exec", r#"{"x":1}"#);
            terminal["output"] = json!([native("exec", r#"{"x":1}"#), message()]);
            let mut body = String::new();
            if include_native_added {
                body.push_str(&frame("response.output_item.added",json!({"type":"response.output_item.added","response_id":"resp_1","output_index":0,"item":native("exec",r#"{"x":1}"#)})));
            }
            append_message_lifecycle(&mut body, 1);
            body.push_str(&frame(
                "response.completed",
                json!({"type":"response.completed","response":terminal}),
            ));
            let (_, bytes) = transform(
                &context(&relay, true),
                true,
                "text/event-stream",
                body.as_bytes(),
            )
            .unwrap();
            let events = events(&bytes);
            let added: Vec<_> = events
                .iter()
                .filter(|event| event["type"] == "response.output_item.added")
                .collect();
            assert_eq!(added.len(), 2);
            assert_eq!(added[0]["output_index"], 0);
            assert_eq!(added[0]["item"]["name"], "exec");
            assert_eq!(added[1]["output_index"], 1);
            assert_eq!(added[1]["item"]["id"], "msg_1");
            assert_eq!(
                events
                    .iter()
                    .filter(|event| event["type"] == "response.function_call_arguments.done")
                    .count(),
                1
            );
            assert_sequences(&events);
            assert_sdk_output(&events);
        }
    }

    fn append_message_lifecycle(body: &mut String, index: usize) {
        let mut added = message();
        added["status"] = json!("in_progress");
        added["content"] = json!([]);
        body.push_str(&frame("response.output_item.added",json!({"type":"response.output_item.added","response_id":"resp_1","output_index":index,"item":added})));
        let part = json!({"type":"output_text","text":"","annotations":[]});
        body.push_str(&frame("response.content_part.added",json!({"type":"response.content_part.added","response_id":"resp_1","output_index":index,"item_id":"msg_1","content_index":0,"part":part})));
        body.push_str(&frame("response.output_text.delta",json!({"type":"response.output_text.delta","response_id":"resp_1","output_index":index,"item_id":"msg_1","content_index":0,"delta":"Working"})));
        body.push_str(&frame("response.output_text.done",json!({"type":"response.output_text.done","response_id":"resp_1","output_index":index,"item_id":"msg_1","content_index":0,"text":"Working"})));
        body.push_str(&frame("response.content_part.done",json!({"type":"response.content_part.done","response_id":"resp_1","output_index":index,"item_id":"msg_1","content_index":0,"part":message()["content"][0]})));
        body.push_str(&frame("response.output_item.done",json!({"type":"response.output_item.done","response_id":"resp_1","output_index":index,"item":message()})));
    }

    #[test]
    fn failed_native_first_stream_compacts_remaining_indexes_for_sdk_consumers() {
        for status in ["failed", "incomplete", "cancelled"] {
            let relay = ToolRelay::new();
            let mut terminal = response(status, "exec", "partial");
            terminal["output"] = json!([native("exec", "partial"), message()]);
            let mut body = frame(
                "response.output_item.added",
                json!({"type":"response.output_item.added","response_id":"resp_1","output_index":0,"item":native("exec","partial")}),
            );
            append_message_lifecycle(&mut body, 1);
            body.push_str(&frame(
                &format!("response.{status}"),
                json!({"type":format!("response.{status}"),"response":terminal}),
            ));
            body.push_str("data: [DONE]\n\n");
            let (_, bytes) = transform(
                &context(&relay, true),
                true,
                "text/event-stream",
                body.as_bytes(),
            )
            .unwrap();
            let events = events(&bytes);
            assert_sdk_output(&events);
            assert_sequences(&events);
            assert!(!String::from_utf8(bytes).unwrap().contains("run_officejs"));
            assert_eq!(relay.cache_len(), 0);
        }
    }
}
