//! Verifies the SGLang request fields survive the router's deserialize/re-serialize hop,
//! and that the worker's extra response fields reach the client untouched.

use axum::{
    body::Body,
    extract::Request,
    http::{header::CONTENT_TYPE, StatusCode},
};
use serde_json::json;
use tower::ServiceExt;

use crate::common::{
    mock_worker::{clear_recorded_requests, recorded_requests, MockWorkerConfig},
    AppTestContext, TestWorkerConfig,
};

async fn post(app: axum::Router, uri: &str, payload: serde_json::Value) -> StatusCode {
    post_for_body(app, uri, payload).await.0
}

async fn post_for_body(
    app: axum::Router,
    uri: &str,
    payload: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_string(&payload).unwrap()))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

fn only_request(port: u16) -> serde_json::Value {
    let requests = recorded_requests(port);
    assert_eq!(
        requests.len(),
        1,
        "worker received {} requests",
        requests.len()
    );
    requests.into_iter().next().unwrap()
}

async fn context(port: u16) -> (AppTestContext, u16) {
    clear_recorded_requests(port);
    let worker: MockWorkerConfig = TestWorkerConfig::healthy(port);
    (AppTestContext::new(vec![worker]).await, port)
}

#[cfg(test)]
mod sglang_field_forwarding_tests {
    use super::*;

    #[tokio::test]
    async fn test_generate_forwards_sglang_fields() {
        let (ctx, port) = context(19950).await;
        let app = ctx.create_app().await;

        let status = post(
            app,
            "/generate",
            json!({
                "input_ids": [1, 2, 3],
                "sampling_params": {"temperature": 0.0, "max_new_tokens": 8},
                "return_logprob": true,
                "return_routed_experts": true,
                "routed_experts_start_len": 0,
                "return_prompt_token_ids": true,
                "return_processor_outputs": true,
                "image_data": [{
                    "format": "processor_output",
                    "pixel_values": {"__tensor__": true, "dtype": "float32", "shape": [1, 2], "data": "AAAAAA=="}
                }],
                "stream": false
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let forwarded = only_request(port);
        assert_eq!(forwarded["return_routed_experts"], json!(true));
        assert_eq!(forwarded["routed_experts_start_len"], json!(0));
        assert_eq!(forwarded["return_prompt_token_ids"], json!(true));
        assert_eq!(forwarded["return_processor_outputs"], json!(true));
        assert_eq!(forwarded["image_data"][0]["format"], "processor_output");
        assert_eq!(
            forwarded["image_data"][0]["pixel_values"]["__tensor__"],
            json!(true)
        );

        ctx.shutdown().await;
    }

    #[tokio::test]
    async fn test_generate_omits_unset_sglang_fields() {
        let (ctx, port) = context(19951).await;
        let app = ctx.create_app().await;

        let status = post(app, "/generate", json!({"text": "hi", "stream": false})).await;
        assert_eq!(status, StatusCode::OK);

        let forwarded = only_request(port);
        for field in [
            "return_routed_experts",
            "routed_experts_start_len",
            "return_prompt_token_ids",
            "return_processor_outputs",
        ] {
            assert!(forwarded.get(field).is_none(), "{field} should not be sent");
        }

        ctx.shutdown().await;
    }

    #[tokio::test]
    async fn test_chat_completions_forwards_sglang_fields() {
        let (ctx, port) = context(19952).await;
        let app = ctx.create_app().await;

        let status = post(
            app,
            "/v1/chat/completions",
            json!({
                "model": "test-model",
                "messages": [{"role": "user", "content": "2+2?"}],
                "logprobs": true,
                "return_prompt_token_ids": true,
                "return_meta_info": true,
                "return_routed_experts": true,
                "routed_experts_start_len": 0,
                "return_cached_tokens_details": true,
                "return_processor_outputs": true,
                "stream": false
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let forwarded = only_request(port);
        for field in [
            "return_prompt_token_ids",
            "return_meta_info",
            "return_routed_experts",
            "return_cached_tokens_details",
            "return_processor_outputs",
        ] {
            assert_eq!(forwarded[field], json!(true), "{field}");
        }
        assert_eq!(forwarded["routed_experts_start_len"], json!(0));

        ctx.shutdown().await;
    }

    #[tokio::test]
    async fn test_generate_returns_worker_extras_to_client() {
        let (ctx, _port) = context(19954).await;
        let app = ctx.create_app().await;

        let (status, body) = post_for_body(
            app,
            "/generate",
            json!({
                "input_ids": [1, 2, 3],
                "return_logprob": true,
                "return_routed_experts": true,
                "return_prompt_token_ids": true,
                "return_processor_outputs": true,
                "stream": false
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        assert_eq!(body["prompt_token_ids"], json!([1, 2, 3]));
        assert_eq!(body["meta_info"]["routed_experts"], json!("AAECAw=="));
        assert_eq!(
            body["meta_info"]["output_token_logprobs"],
            json!([[-0.5, 42, null]])
        );
        assert_eq!(
            body["processor_outputs"]["pixel_values"]["dtype"],
            json!("float32")
        );

        ctx.shutdown().await;
    }

    #[tokio::test]
    async fn test_chat_completions_returns_worker_extras_to_client() {
        let (ctx, _port) = context(19955).await;
        let app = ctx.create_app().await;

        let (status, body) = post_for_body(
            app,
            "/v1/chat/completions",
            json!({
                "model": "test-model",
                "messages": [{"role": "user", "content": "2+2?"}],
                "logprobs": true,
                "return_prompt_token_ids": true,
                "return_meta_info": true,
                "return_routed_experts": true,
                "return_processor_outputs": true,
                "stream": false
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        assert_eq!(body["choices"][0]["prompt_token_ids"], json!([1, 2, 3]));
        assert_eq!(
            body["choices"][0]["meta_info"]["output_token_logprobs"],
            json!([[-0.5, 42, null]])
        );
        assert_eq!(body["sglext"]["routed_experts"], json!("AAECAw=="));
        assert_eq!(
            body["sglext"]["processor_outputs"]["pixel_values"]["__tensor__"],
            json!(true)
        );

        ctx.shutdown().await;
    }

    #[tokio::test]
    async fn test_chat_completions_forwards_object_tool_call_arguments() {
        let (ctx, port) = context(19953).await;
        let app = ctx.create_app().await;

        let status = post(
            app,
            "/v1/chat/completions",
            json!({
                "model": "test-model",
                "messages": [
                    {"role": "user", "content": "2+2?"},
                    {"role": "assistant", "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {"name": "calculator", "arguments": {"expression": "2+2"}}
                    }]},
                    {"role": "tool", "tool_call_id": "call_1", "content": "4"}
                ],
                "stream": false
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let forwarded = only_request(port);
        assert_eq!(
            forwarded["messages"][1]["tool_calls"][0]["function"]["arguments"],
            json!(r#"{"expression":"2+2"}"#)
        );

        ctx.shutdown().await;
    }
}
