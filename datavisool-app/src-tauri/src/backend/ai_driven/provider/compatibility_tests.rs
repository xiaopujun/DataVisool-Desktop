/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Rig Provider 请求级兼容门禁。
//!
//! 测试只连接进程内 mock HTTP 服务，覆盖真实请求序列化与响应解析，禁止访问公网。

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{self, Receiver};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rig_core::agent::{stream_to_stdout, AgentBuilder};
use rig_core::completion::{AssistantContent, CompletionModel, Message, ToolDefinition};
use rig_core::message::{ImageMediaType, UserContent};
use rig_core::streaming::StreamingPrompt;
use rig_core::OneOrMany;
use serde_json::{json, Value};

use super::{RigCompletionModel, RigModelFactory};

const TEST_KEY: &str = "compat-secret";
const TEST_IMAGE: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAAB";

#[derive(Debug)]
struct CapturedRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Value,
}

struct MockReply {
    status: u16,
    content_type: &'static str,
    body: String,
    delay: Duration,
}

impl MockReply {
    fn json(body: Value) -> Self {
        Self { status: 200, content_type: "application/json", body: body.to_string(), delay: Duration::ZERO }
    }

    fn sse(events: impl IntoIterator<Item = Value>) -> Self {
        let mut body = events.into_iter().map(|event| format!("data: {event}\n\n")).collect::<String>();
        body.push_str("data: [DONE]\n\n");
        Self { status: 200, content_type: "text/event-stream", body, delay: Duration::ZERO }
    }

    fn error(status: u16, message: &str) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: json!({ "error": { "message": message, "type": "compat_test" } }).to_string(),
            delay: Duration::ZERO,
        }
    }

    fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }
}

struct MockServer {
    origin: String,
    requests: Receiver<CapturedRequest>,
    worker: JoinHandle<()>,
}

impl MockServer {
    fn start(replies: Vec<MockReply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("mock listener should bind");
        let address = listener.local_addr().expect("mock listener should have address");
        let (sender, requests) = mpsc::channel();
        let worker = thread::spawn(move || {
            for reply in replies {
                let (mut stream, _) = listener.accept().expect("mock request should connect");
                let request = read_request(&mut stream);
                sender.send(request).expect("test should receive captured request");
                if !reply.delay.is_zero() {
                    thread::sleep(reply.delay);
                }
                write_reply(&mut stream, &reply);
            }
        });

        Self { origin: format!("http://{address}"), requests, worker }
    }

    fn openai_base_url(&self) -> String {
        format!("{}/v1", self.origin)
    }

    fn anthropic_messages_url(&self) -> String {
        format!("{}/v1/messages", self.origin)
    }

    fn gemini_base_url(&self) -> String {
        self.origin.clone()
    }

    fn xai_base_url(&self) -> String {
        self.origin.clone()
    }

    fn take_request(&self) -> CapturedRequest {
        self.requests
            .recv_timeout(Duration::from_secs(3))
            .expect("provider should issue a request")
    }

    fn finish(self) {
        self.worker.join().expect("mock worker should finish");
    }
}

fn read_request(stream: &mut TcpStream) -> CapturedRequest {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("read timeout should configure");
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let read = stream.read(&mut chunk).expect("request bytes should read");
        assert!(read > 0, "request closed before headers completed");
        bytes.extend_from_slice(&chunk[..read]);
        if let Some(index) = find_header_end(&bytes) {
            break index;
        }
    };

    let header_text = String::from_utf8(bytes[..header_end].to_vec()).expect("headers should be UTF-8");
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().expect("request line should exist");
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().expect("method should exist").to_string();
    let path = request_parts.next().expect("path should exist").to_string();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect::<HashMap<_, _>>();
    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    let body_start = header_end + 4;
    while bytes.len() < body_start + content_length {
        let read = stream.read(&mut chunk).expect("request body should read");
        assert!(read > 0, "request closed before body completed");
        bytes.extend_from_slice(&chunk[..read]);
    }
    let body = if content_length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[body_start..body_start + content_length]).expect("request body should be JSON")
    };

    CapturedRequest { method, path, headers, body }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes.windows(4).position(|window| window == b"\r\n\r\n")
}

fn write_reply(stream: &mut TcpStream, reply: &MockReply) {
    let reason = match reply.status {
        200 => "OK",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Mock Error",
    };
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reply.status,
        reason,
        reply.content_type,
        reply.body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(reply.body.as_bytes());
    let _ = stream.flush();
}

fn multimodal_prompt() -> Message {
    Message::User {
        content: OneOrMany::many(vec![
            UserContent::text("inspect this image"),
            UserContent::image_base64(TEST_IMAGE, Some(ImageMediaType::PNG), None),
        ])
        .expect("prompt has content"),
    }
}

fn tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "lookup".to_string(),
            description: "look up a value".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "key": { "type": "string" } },
                "required": ["key"]
            }),
        },
        ToolDefinition {
            name: "save".to_string(),
            description: "save a value".to_string(),
            parameters: json!({
                "type": "object",
                "properties": { "value": { "type": "integer" } },
                "required": ["value"]
            }),
        },
    ]
}

fn output_schema() -> rig_core::schemars::Schema {
    serde_json::from_value(json!({
        "title": "CompatResult",
        "type": "object",
        "properties": { "answer": { "type": "string" } },
        "required": ["answer"]
    }))
    .expect("schema should deserialize")
}

fn openai_response(content: &str, finish_reason: &str) -> Value {
    json!({
        "id": "chatcmpl-compat",
        "object": "chat.completion",
        "created": 1,
        "model": "compat-model",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": content },
            "logprobs": null,
            "finish_reason": finish_reason
        }],
        "usage": { "prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10 }
    })
}

fn gemini_interaction_response(content: &str) -> Value {
    json!({
        "id": "interaction-compat",
        "status": "completed",
        "model": "gemini-3.6-flash",
        "steps": [{
            "type": "model_output",
            "content": [{ "type": "text", "text": content }]
        }],
        "usage": {
            "total_tokens": 10,
            "total_input_tokens": 7,
            "total_output_tokens": 3
        }
    })
}

fn xai_response(content: &str) -> Value {
    json!({
        "id": "resp-compat",
        "model": "grok-4.6",
        "output": [{
            "type": "message",
            "id": "msg-compat",
            "role": "assistant",
            "status": "completed",
            "content": [{ "type": "output_text", "text": content, "annotations": [] }]
        }],
        "usage": {
            "input_tokens": 7,
            "output_tokens": 3,
            "total_tokens": 10
        }
    })
}

fn openai_tool_response(calls: Value) -> Value {
    json!({
        "id": "chatcmpl-tools",
        "object": "chat.completion",
        "created": 1,
        "model": "compat-model",
        "system_fingerprint": null,
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": [], "tool_calls": calls },
            "logprobs": null,
            "finish_reason": "tool_calls"
        }],
        "usage": { "prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10 }
    })
}

fn assert_bearer_request(request: &CapturedRequest) {
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(request.headers.get("authorization").map(String::as_str), Some("Bearer compat-secret"));
}

fn parsed_tool_calls(choice: &OneOrMany<AssistantContent>) -> Vec<(&str, &str, Value)> {
    choice
        .iter()
        .filter_map(|content| match content {
            AssistantContent::ToolCall(call) => Some((call.id.as_str(), call.function.name.as_str(), call.function.arguments.clone())),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn openai_supports_multimodal_tools_structured_output_and_usage() {
    let calls = json!([
        { "id": "call-1", "type": "function", "function": { "name": "lookup", "arguments": "{\"key\":\"alpha\"}" } },
        { "id": "call-2", "type": "function", "function": { "name": "save", "arguments": "{\"value\":2}" } }
    ]);
    let server = MockServer::start(vec![
        MockReply::json(openai_tool_response(calls)),
        MockReply::json(openai_response("{\"answer\":\"ok\"}", "stop")),
    ]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("openai", Some(TEST_KEY), &server.openai_base_url(), "gpt-compat")
        .unwrap();
    let RigCompletionModel::OpenAi(model) = built.into_model() else {
        panic!("expected OpenAI model");
    };

    let tool_response = model
        .completion_request(multimodal_prompt())
        .tools(tools())
        .max_tokens(128)
        .send()
        .await
        .expect("OpenAI tool response should parse");
    assert_eq!(
        parsed_tool_calls(&tool_response.choice),
        vec![("call-1", "lookup", json!({ "key": "alpha" })), ("call-2", "save", json!({ "value": 2 })),]
    );
    assert_eq!(
        (tool_response.usage.input_tokens, tool_response.usage.output_tokens, tool_response.usage.total_tokens),
        (7, 3, 10)
    );
    assert_eq!(tool_response.raw_response.choices[0].finish_reason, "tool_calls");

    let tool_request = server.take_request();
    assert_bearer_request(&tool_request);
    assert_eq!(tool_request.body["model"], "gpt-compat");
    assert_eq!(tool_request.body["messages"][0]["content"][0]["text"], "inspect this image");
    assert_eq!(tool_request.body["messages"][0]["content"][1]["type"], "image_url");
    assert_eq!(
        tool_request.body["messages"][0]["content"][1]["image_url"]["url"],
        format!("data:image/png;base64,{TEST_IMAGE}")
    );
    assert_eq!(tool_request.body["tools"][0]["function"]["parameters"]["required"][0], "key");

    let structured_response = model
        .completion_request(Message::user("return JSON"))
        .output_schema(output_schema())
        .send()
        .await
        .expect("OpenAI structured response should parse");
    assert!(matches!(structured_response.choice.first(), AssistantContent::Text(text) if text.text == "{\"answer\":\"ok\"}"));
    assert_eq!(structured_response.raw_response.choices[0].finish_reason, "stop");

    let structured_request = server.take_request();
    assert_eq!(structured_request.body["response_format"]["type"], "json_schema");
    assert_eq!(structured_request.body["response_format"]["json_schema"]["name"], "CompatResult");
    assert_eq!(structured_request.body["response_format"]["json_schema"]["strict"], true);
    server.finish();
}

#[tokio::test]
async fn qwen_uses_openai_compatible_chat_completions() {
    let server = MockServer::start(vec![MockReply::json(openai_response("qwen response", "stop"))]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("qwen", Some(TEST_KEY), &server.openai_base_url(), "qwen3.7-plus")
        .unwrap();

    assert_eq!(built.provider().as_str(), "qwen");
    let RigCompletionModel::OpenAi(model) = built.into_model() else {
        panic!("expected OpenAI-compatible Qwen model");
    };
    let response = model
        .completion_request(Message::user("hello"))
        .send()
        .await
        .expect("Qwen response should parse");

    assert!(matches!(response.choice.first(), AssistantContent::Text(text) if text.text == "qwen response"));
    let request = server.take_request();
    assert_bearer_request(&request);
    assert_eq!(request.body["model"], "qwen3.7-plus");
    server.finish();
}

#[tokio::test]
async fn kimi_uses_openai_compatible_chat_completions() {
    let server = MockServer::start(vec![MockReply::json(openai_response("kimi response", "stop"))]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("kimi", Some(TEST_KEY), &server.openai_base_url(), "kimi-k3")
        .unwrap();

    assert_eq!(built.provider().as_str(), "kimi");
    let RigCompletionModel::OpenAi(model) = built.into_model() else {
        panic!("expected OpenAI-compatible Kimi model");
    };
    let response = model
        .completion_request(Message::user("hello"))
        .send()
        .await
        .expect("Kimi response should parse");

    assert!(matches!(response.choice.first(), AssistantContent::Text(text) if text.text == "kimi response"));
    let request = server.take_request();
    assert_bearer_request(&request);
    assert_eq!(request.body["model"], "kimi-k3");
    server.finish();
}

#[tokio::test]
async fn glm_uses_openai_compatible_chat_completions() {
    let server = MockServer::start(vec![MockReply::json(openai_response("glm response", "stop"))]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("glm", Some(TEST_KEY), &server.openai_base_url(), "glm-5.2")
        .unwrap();

    assert_eq!(built.provider().as_str(), "glm");
    let RigCompletionModel::OpenAi(model) = built.into_model() else {
        panic!("expected OpenAI-compatible GLM model");
    };
    let response = model
        .completion_request(Message::user("hello"))
        .send()
        .await
        .expect("GLM response should parse");

    assert!(matches!(response.choice.first(), AssistantContent::Text(text) if text.text == "glm response"));
    let request = server.take_request();
    assert_bearer_request(&request);
    assert_eq!(request.body["model"], "glm-5.2");
    server.finish();
}

#[tokio::test]
async fn minimax_uses_openai_compatible_chat_completions() {
    let server = MockServer::start(vec![MockReply::json(openai_response("MiniMax response", "stop"))]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("minimax", Some(TEST_KEY), &server.openai_base_url(), "MiniMax-M3")
        .unwrap();

    assert_eq!(built.provider().as_str(), "minimax");
    let RigCompletionModel::OpenAi(model) = built.into_model() else {
        panic!("expected OpenAI-compatible MiniMax model");
    };
    let response = model
        .completion_request(Message::user("hello"))
        .send()
        .await
        .expect("MiniMax response should parse");

    assert!(matches!(response.choice.first(), AssistantContent::Text(text) if text.text == "MiniMax response"));
    let request = server.take_request();
    assert_bearer_request(&request);
    assert_eq!(request.body["model"], "MiniMax-M3");
    server.finish();
}

#[tokio::test]
async fn doubao_uses_openai_compatible_chat_completions() {
    let server = MockServer::start(vec![MockReply::json(openai_response("Doubao response", "stop"))]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("doubao", Some(TEST_KEY), &server.openai_base_url(), "doubao-seed-2-0-lite-260215")
        .unwrap();

    assert_eq!(built.provider().as_str(), "doubao");
    let RigCompletionModel::OpenAi(model) = built.into_model() else {
        panic!("expected OpenAI-compatible Doubao model");
    };
    let response = model
        .completion_request(Message::user("hello"))
        .send()
        .await
        .expect("Doubao response should parse");

    assert!(matches!(response.choice.first(), AssistantContent::Text(text) if text.text == "Doubao response"));
    let request = server.take_request();
    assert_bearer_request(&request);
    assert_eq!(request.body["model"], "doubao-seed-2-0-lite-260215");
    server.finish();
}

#[tokio::test]
async fn hunyuan_uses_openai_compatible_chat_completions() {
    let server = MockServer::start(vec![MockReply::json(openai_response("Hunyuan response", "stop"))]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("hunyuan", Some(TEST_KEY), &server.openai_base_url(), "hunyuan-turbos-latest")
        .unwrap();

    assert_eq!(built.provider().as_str(), "hunyuan");
    let RigCompletionModel::OpenAi(model) = built.into_model() else {
        panic!("expected OpenAI-compatible Hunyuan model");
    };
    let response = model
        .completion_request(Message::user("hello"))
        .send()
        .await
        .expect("Hunyuan response should parse");

    assert!(matches!(response.choice.first(), AssistantContent::Text(text) if text.text == "Hunyuan response"));
    let request = server.take_request();
    assert_bearer_request(&request);
    assert_eq!(request.body["model"], "hunyuan-turbos-latest");
    server.finish();
}

#[tokio::test]
async fn gemini_uses_interactions_api() {
    let server = MockServer::start(vec![MockReply::json(gemini_interaction_response("Gemini response"))]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("gemini", Some(TEST_KEY), &server.gemini_base_url(), "gemini-3.6-flash")
        .unwrap();

    assert_eq!(built.provider().as_str(), "gemini");
    let RigCompletionModel::Gemini(model) = built.into_model() else {
        panic!("expected Gemini interactions model");
    };
    let response = model
        .completion_request(Message::user("hello"))
        .send()
        .await
        .expect("Gemini interaction response should parse");

    assert!(matches!(response.choice.first(), AssistantContent::Text(text) if text.text == "Gemini response"));
    let request = server.take_request();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1beta/interactions");
    assert_eq!(request.headers.get("x-goog-api-key").map(String::as_str), Some(TEST_KEY));
    assert_eq!(request.body["model"], "gemini-3.6-flash");
    assert_eq!(request.body["stream"], false);
    assert!(request.body["generation_config"]["temperature"].is_null());
    server.finish();
}

#[tokio::test]
async fn grok_uses_xai_responses_api() {
    let server = MockServer::start(vec![MockReply::json(xai_response("Grok response"))]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("grok", Some(TEST_KEY), &server.xai_base_url(), "grok-4.6")
        .unwrap();

    assert_eq!(built.provider().as_str(), "grok");
    let RigCompletionModel::Grok(model) = built.into_model() else {
        panic!("expected xAI Grok responses model");
    };
    let response = model
        .completion_request(Message::user("hello"))
        .send()
        .await
        .expect("Grok response should parse");

    assert!(matches!(response.choice.first(), AssistantContent::Text(text) if text.text == "Grok response"));
    let request = server.take_request();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/responses");
    assert_eq!(request.headers.get("authorization").map(String::as_str), Some("Bearer compat-secret"));
    assert_eq!(request.body["model"], "grok-4.6");
    assert_eq!(request.body["input"][0]["role"], "user");
    server.finish();
}

#[tokio::test]
async fn deepseek_filters_images_uses_json_object_and_parses_one_tool_call() {
    let response = json!({
        "id": "deepseek-compat",
        "model": "deepseek-chat",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": {
                "role": "assistant",
                "content": "",
                "tool_calls": [{
                    "id": "deep-call",
                    "index": 0,
                    "type": "function",
                    "function": { "name": "lookup", "arguments": "{\"key\":\"beta\"}" }
                }]
            },
            "logprobs": null,
            "finish_reason": "tool_calls"
        }],
        "usage": {
            "prompt_tokens": 11,
            "completion_tokens": 4,
            "prompt_cache_hit_tokens": 0,
            "prompt_cache_miss_tokens": 11,
            "total_tokens": 15,
            "prompt_tokens_details": { "cached_tokens": 2 }
        }
    });
    let server = MockServer::start(vec![MockReply::json(response)]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("deepseek", Some(TEST_KEY), &server.openai_base_url(), "deepseek-chat")
        .unwrap();
    let RigCompletionModel::DeepSeek(model) = built.into_model() else {
        panic!("expected DeepSeek model");
    };

    let response = model
        .completion_request(multimodal_prompt())
        .tools(vec![tools().remove(0)])
        .output_schema(output_schema())
        .additional_params(json!({ "response_format": { "type": "json_object" } }))
        .send()
        .await
        .expect("DeepSeek response should parse");
    assert_eq!(parsed_tool_calls(&response.choice), vec![("deep-call", "lookup", json!({ "key": "beta" }))]);
    assert_eq!((response.usage.input_tokens, response.usage.output_tokens, response.usage.total_tokens), (11, 4, 15));
    assert_eq!(response.usage.cached_input_tokens, 2);
    assert_eq!(response.raw_response.choices[0].finish_reason, "tool_calls");

    let request = server.take_request();
    assert_bearer_request(&request);
    assert_eq!(request.body["messages"][0]["content"], "inspect this image");
    assert_eq!(request.body["response_format"]["type"], "json_object");
    assert_eq!(request.body["tools"][0]["function"]["name"], "lookup");
    server.finish();
}

#[tokio::test]
async fn mimo_supports_multimodal_and_json_schema_on_custom_base_url() {
    let server = MockServer::start(vec![MockReply::json(openai_response("{\"answer\":\"mimo\"}", "stop"))]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("mimo", Some(TEST_KEY), &server.openai_base_url(), "mimo-v2-omni")
        .unwrap();
    assert_eq!(built.base_url(), server.openai_base_url());
    let RigCompletionModel::Mimo(model) = built.into_model() else {
        panic!("expected MiMo model");
    };

    let response = model
        .completion_request(multimodal_prompt())
        .output_schema(output_schema())
        .send()
        .await
        .expect("MiMo response should parse");
    assert_eq!((response.usage.input_tokens, response.usage.output_tokens), (7, 3));
    assert_eq!(response.raw_response.choices[0].finish_reason, "stop");

    let request = server.take_request();
    assert_bearer_request(&request);
    assert_eq!(request.body["model"], "mimo-v2-omni");
    assert_eq!(request.body["messages"][0]["content"][1]["type"], "image_url");
    assert_eq!(request.body["response_format"]["type"], "json_schema");
    assert!(request.body.get("max_tokens").is_none(), "unset maxTokens must be omitted");
    assert!(request.body.get("max_completion_tokens").is_none(), "unset maxTokens must not create a provider cap");
    server.finish();
}

#[tokio::test]
async fn anthropic_supports_multimodal_multi_tools_schema_usage_and_headers() {
    let response = json!({
        "type": "message",
        "id": "msg-compat",
        "model": "claude-compat",
        "role": "assistant",
        "content": [
            { "type": "tool_use", "id": "anth-call-1", "name": "lookup", "input": { "key": "gamma" } },
            { "type": "tool_use", "id": "anth-call-2", "name": "save", "input": { "value": 3 } }
        ],
        "stop_reason": "tool_use",
        "stop_sequence": null,
        "usage": {
            "input_tokens": 13,
            "cache_read_input_tokens": 2,
            "cache_creation_input_tokens": 1,
            "output_tokens": 5
        }
    });
    let server = MockServer::start(vec![MockReply::json(response)]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("anthropic", Some(TEST_KEY), &server.anthropic_messages_url(), "claude-compat")
        .unwrap();
    assert_eq!(built.base_url(), server.origin);
    let RigCompletionModel::Anthropic(model) = built.into_model() else {
        panic!("expected Anthropic model");
    };

    let response = model
        .completion_request(multimodal_prompt())
        .tools(tools())
        .output_schema(output_schema())
        .max_tokens(128)
        .send()
        .await
        .expect("Anthropic response should parse");
    assert_eq!(
        parsed_tool_calls(&response.choice),
        vec![("anth-call-1", "lookup", json!({ "key": "gamma" })), ("anth-call-2", "save", json!({ "value": 3 })),]
    );
    assert_eq!((response.usage.input_tokens, response.usage.output_tokens, response.usage.total_tokens), (13, 5, 21));
    assert_eq!((response.usage.cached_input_tokens, response.usage.cache_creation_input_tokens), (2, 1));
    assert_eq!(response.raw_response.stop_reason.as_deref(), Some("tool_use"));

    let request = server.take_request();
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/messages");
    assert_eq!(request.headers.get("x-api-key").map(String::as_str), Some(TEST_KEY));
    assert_eq!(request.headers.get("anthropic-version").map(String::as_str), Some("2023-06-01"));
    assert_eq!(request.body["messages"][0]["content"][0]["text"], "inspect this image");
    assert_eq!(request.body["messages"][0]["content"][1]["source"]["type"], "base64");
    assert_eq!(request.body["messages"][0]["content"][1]["source"]["media_type"], "image/png");
    assert_eq!(request.body["tools"][0]["input_schema"]["required"][0], "key");
    assert_eq!(request.body["output_config"]["format"]["type"], "json_schema");
    assert_eq!(request.body["output_config"]["format"]["schema"]["properties"]["answer"]["type"], "string");
    server.finish();
}

#[tokio::test]
async fn openai_compatible_streaming_sse_is_aggregated_with_usage() {
    let server = MockServer::start(vec![MockReply::sse([
        json!({
            "id": "stream-openai",
            "model": "gpt-stream",
            "choices": [{ "delta": { "content": "hel", "tool_calls": [] }, "finish_reason": null }],
            "usage": null
        }),
        json!({
            "id": "stream-openai",
            "model": "gpt-stream",
            "choices": [{ "delta": { "content": "lo", "tool_calls": [] }, "finish_reason": "stop" }],
            "usage": null
        }),
        json!({ "choices": [], "usage": { "prompt_tokens": 4, "completion_tokens": 2, "total_tokens": 6 } }),
    ])]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("openai", Some(TEST_KEY), &server.openai_base_url(), "gpt-stream")
        .unwrap();
    let RigCompletionModel::OpenAi(model) = built.into_model() else {
        panic!("expected OpenAI model");
    };
    let agent = AgentBuilder::new(model).build();

    let mut stream = agent.stream_prompt("stream please").await;
    let response = stream_to_stdout(&mut stream).await.expect("stream should consume");
    assert_eq!(response.output(), "hello");
    assert_eq!((response.usage().input_tokens, response.usage().output_tokens, response.usage().total_tokens), (4, 2, 6));

    let request = server.take_request();
    assert_eq!(request.body["stream"], true);
    assert_eq!(request.body["stream_options"]["include_usage"], true);
    server.finish();
}

#[tokio::test]
async fn anthropic_streaming_sse_is_aggregated_with_usage() {
    let server = MockServer::start(vec![MockReply::sse([
        json!({
            "type": "message_start",
            "message": {
                "id": "msg-stream",
                "role": "assistant",
                "content": [],
                "model": "claude-stream",
                "stop_reason": null,
                "stop_sequence": null,
                "usage": { "input_tokens": 6, "output_tokens": 0 }
            }
        }),
        json!({ "type": "content_block_start", "index": 0, "content_block": { "type": "text", "text": "", "citations": [] } }),
        json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "anth" } }),
        json!({ "type": "content_block_delta", "index": 0, "delta": { "type": "text_delta", "text": "ropic" } }),
        json!({ "type": "content_block_stop", "index": 0 }),
        json!({
            "type": "message_delta",
            "delta": { "stop_reason": "end_turn", "stop_sequence": null },
            "usage": { "output_tokens": 3, "cache_creation_input_tokens": 1, "cache_read_input_tokens": 2 }
        }),
    ])]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("anthropic", Some(TEST_KEY), &server.anthropic_messages_url(), "claude-stream")
        .unwrap();
    let RigCompletionModel::Anthropic(model) = built.into_model() else {
        panic!("expected Anthropic model");
    };
    let agent = AgentBuilder::new(model).max_tokens(64).build();

    let mut stream = agent.stream_prompt("stream please").await;
    let response = stream_to_stdout(&mut stream).await.expect("stream should consume");
    assert_eq!(response.output(), "anthropic");
    assert_eq!((response.usage().input_tokens, response.usage().output_tokens, response.usage().total_tokens), (6, 3, 12));
    assert_eq!((response.usage().cached_input_tokens, response.usage().cache_creation_input_tokens), (2, 1));

    let request = server.take_request();
    assert_eq!(request.body["stream"], true);
    assert_eq!(request.body["max_tokens"], 64);
    server.finish();
}

#[tokio::test]
async fn provider_content_filter_finish_reasons_are_preserved() {
    let openai_server = MockServer::start(vec![MockReply::json(openai_response("filtered", "content_filter"))]);
    let openai = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("openai", Some(TEST_KEY), &openai_server.openai_base_url(), "gpt-filter")
        .unwrap();
    let RigCompletionModel::OpenAi(openai) = openai.into_model() else {
        panic!("expected OpenAI model");
    };
    let openai_response = openai
        .completion_request(Message::user("filtered request"))
        .send()
        .await
        .expect("content-filtered OpenAI response should remain inspectable");
    assert_eq!(openai_response.raw_response.choices[0].finish_reason, "content_filter");
    openai_server.take_request();
    openai_server.finish();

    let anthropic_server = MockServer::start(vec![MockReply::json(json!({
        "type": "message",
        "id": "msg-refusal",
        "model": "claude-filter",
        "role": "assistant",
        "content": [{ "type": "text", "text": "refused" }],
        "stop_reason": "refusal",
        "stop_sequence": null,
        "usage": { "input_tokens": 2, "output_tokens": 1 }
    }))]);
    let anthropic = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("anthropic", Some(TEST_KEY), &anthropic_server.anthropic_messages_url(), "claude-filter")
        .unwrap();
    let RigCompletionModel::Anthropic(anthropic) = anthropic.into_model() else {
        panic!("expected Anthropic model");
    };
    let anthropic_response = anthropic
        .completion_request(Message::user("filtered request"))
        .max_tokens(16)
        .send()
        .await
        .expect("refusal response should remain inspectable");
    assert_eq!(anthropic_response.raw_response.stop_reason.as_deref(), Some("refusal"));
    anthropic_server.take_request();
    anthropic_server.finish();
}

#[tokio::test]
async fn http_errors_are_not_retried_by_provider_layer() {
    let server = MockServer::start(vec![MockReply::error(429, "compat rate limit")]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("openai", Some(TEST_KEY), &server.openai_base_url(), "gpt-error")
        .unwrap();
    let RigCompletionModel::OpenAi(model) = built.into_model() else {
        panic!("expected OpenAI model");
    };

    let error = model
        .completion_request(Message::user("fail once"))
        .send()
        .await
        .expect_err("HTTP error should propagate");
    let message = error.to_string();
    assert!(message.contains("429") || message.contains("compat rate limit"), "unexpected error: {message}");
    let request = server.take_request();
    assert_eq!(request.path, "/v1/chat/completions");
    assert!(server.requests.recv_timeout(Duration::from_millis(100)).is_err(), "provider layer must not retry");
    server.finish();
}

#[tokio::test]
async fn caller_timeout_cancels_in_flight_completion_without_retry() {
    let server = MockServer::start(vec![MockReply::json(openai_response("late", "stop")).delayed(Duration::from_millis(150))]);
    let built = RigModelFactory::new()
        .unwrap()
        .build_from_runtime_parts("openai", Some(TEST_KEY), &server.openai_base_url(), "gpt-slow")
        .unwrap();
    let RigCompletionModel::OpenAi(model) = built.into_model() else {
        panic!("expected OpenAI model");
    };

    let result = tokio::time::timeout(Duration::from_millis(30), model.completion_request(Message::user("too slow")).send()).await;
    assert!(result.is_err(), "caller timeout should cancel the request future");
    let request = server.take_request();
    assert_eq!(request.path, "/v1/chat/completions");
    assert!(server.requests.recv_timeout(Duration::from_millis(50)).is_err(), "cancelled request must not retry");
    server.finish();
}
