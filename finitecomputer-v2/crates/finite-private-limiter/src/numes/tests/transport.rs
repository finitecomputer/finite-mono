use super::*;

#[test]
fn streamed_usage_must_be_final_and_unambiguous() {
    let usage = format!(
        "data: {}\n\n",
        json!({"model":MODEL,"usage":{
            "prompt_tokens":10,"completion_tokens":2,"total_tokens":12
        }})
    );
    let content = format!(
        "data: {}\n\n",
        json!({"model":MODEL,"choices":[{
            "delta":{"content":"more output"}
        }]})
    );
    let done = "data: [DONE]\n\n";
    for wire in [
        format!("{usage}{content}{done}"),
        format!("{usage}{usage}{done}"),
        format!("{usage}{done}{content}"),
        format!("{usage}{done}{done}"),
        format!("{usage}{done}data: {{\"unfinished\":"),
    ] {
        let mut parser = StreamUsage::new(MODEL, 64);
        for chunk in wire.as_bytes().chunks(3) {
            parser.push(chunk);
        }
        assert!(
            parser.finish().is_none(),
            "accepted ambiguous usage: {wire}"
        );
    }
}

#[tokio::test]
async fn receipt_and_capacity_wait_for_normal_upstream_eof_after_done() {
    let gate = Arc::new(Semaphore::new(0));
    let mut engine = Engine::new();
    engine.after_done = Some(gate.clone());
    engine.release.add_permits(1);
    let h = harness(engine, Duration::from_secs(5)).await;
    let response = h.app.clone().oneshot(request(KEY, true)).await.unwrap();
    let mut body = response.into_body();
    let mut received = Vec::new();
    while !String::from_utf8_lossy(&received).contains("data: [DONE]") {
        let frame = body.frame().await.unwrap().unwrap();
        received.extend_from_slice(frame.data_ref().unwrap());
    }
    // The SSE terminator alone does not establish successful HTTP completion.
    assert!(
        timeout(Duration::from_millis(50), body.frame())
            .await
            .is_err()
    );
    assert_eq!(inflight(&h.app).await, 1);
    gate.add_permits(1);
    let collected = body.collect().await.unwrap();
    assert_eq!(
        collected.trailers().unwrap()[USAGE_HEADER],
        "prompt=10,completion=2,total=12,model=glm-5-3-flash"
    );
    wait_for(async || inflight(&h.app).await == 0).await;
}

#[tokio::test]
async fn upstream_failure_or_timeout_after_done_cannot_emit_a_receipt() {
    for upstream_fails in [false, true] {
        let gate = Arc::new(Semaphore::new(0));
        let mut engine = Engine::new();
        engine.after_done = Some(gate.clone());
        engine.fail_after_done = upstream_fails;
        engine.release.add_permits(1);
        let h = harness(engine, Duration::from_millis(200)).await;
        let response = h.app.clone().oneshot(request(KEY, true)).await.unwrap();
        let mut body = response.into_body();
        let mut received = Vec::new();
        while !String::from_utf8_lossy(&received).contains("data: [DONE]") {
            let frame = body.frame().await.unwrap().unwrap();
            received.extend_from_slice(frame.data_ref().unwrap());
        }
        if upstream_fails {
            gate.add_permits(1);
        }
        assert!(body.collect().await.is_err());
        wait_for(async || inflight(&h.app).await == 0).await;
    }
}

#[tokio::test]
async fn minimal_client_body_preserves_cache_secret_and_bounds_output() {
    let h = harness(Engine::new(), Duration::from_secs(5)).await;
    for stream in [None, Some(false), Some(true)] {
        let mut input = json!({
            "model": MODEL,
            "messages": [{"role":"user", "content":"synthetic hello"}],
            "user_cache_secret": "synthetic-conversation-cache-secret"
        });
        if let Some(stream) = stream {
            input["stream"] = json!(stream);
        }
        let mut request = request(KEY, stream == Some(true));
        request.headers_mut().remove("x-request-id");
        *request.body_mut() = Body::from(input.to_string());
        h.engine.release.add_permits(1);
        let response = h.app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        response.into_body().collect().await.unwrap();
        let forwarded = h.engine.last_request.lock().unwrap().clone().unwrap();
        assert_eq!(forwarded["user_cache_secret"], input["user_cache_secret"]);
        assert_eq!(forwarded["messages"], input["messages"]);
        assert_eq!(forwarded["max_tokens"], 64);
        assert_eq!(forwarded["n"], 1);
        wait_for(async || inflight(&h.app).await == 0).await;
    }
    assert_eq!(h.core_calls.load(Ordering::SeqCst), 0);
}
