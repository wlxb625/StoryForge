use std::time::{Duration, Instant};

use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use reqwest::{Client, Response};
use serde::Serialize;
use serde_json::{json, Value};

use crate::{
    db::Metering,
    error::{Result, StudioError},
    profiles::{completion_url, Profile},
};

pub struct Remote {
    system: Client,
    direct: Client,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub state: &'static str,
    pub message: Option<String>,
    pub finish_reason: Option<String>,
    pub status: Option<u16>,
    pub streamed: bool,
    pub usage_observed: bool,
}

impl Outcome {
    fn unknown(message: &str) -> Self {
        Self {
            state: "outcome_unknown",
            message: Some(message.into()),
            finish_reason: None,
            status: None,
            streamed: false,
            usage_observed: false,
        }
    }
}

impl Remote {
    pub fn new() -> Result<Self> {
        let make = || {
            Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .connect_timeout(Duration::from_secs(18))
                .read_timeout(Duration::from_secs(65))
                .user_agent("StoryForge/0.1")
        };
        let unavailable = |_| StudioError::new(500, "无法初始化模型网络连接");
        Ok(Self {
            system: make().build().map_err(unavailable)?,
            direct: make().no_proxy().build().map_err(unavailable)?,
        })
    }

    async fn send(
        &self,
        profile: &Profile,
        secret: Option<&str>,
        body: &Value,
        timeout: Duration,
    ) -> Result<Response> {
        if body["model"].as_str() != Some(profile.model.as_str()) || !body["messages"].is_array() {
            return Err(StudioError::new(400, "冻结的模型参数与本机档案不一致"));
        }
        let client = if profile.direct {
            &self.direct
        } else {
            &self.system
        };
        let mut request = client
            .post(completion_url(&profile.base_url)?)
            .json(body)
            .timeout(timeout);
        if let Some(secret) = secret {
            request = request.bearer_auth(secret);
        }
        request
            .send()
            .await
            .map_err(|_| StudioError::new(502, "模型连接中断，远端结果未知"))
    }

    pub async fn call(
        &self,
        profile: &Profile,
        secret: Option<&str>,
        body: &Value,
        timeout: Duration,
        mut on_text: impl FnMut(&str) -> Result<()>,
    ) -> (Outcome, Metering) {
        let timer = Instant::now();
        let mut metering = Metering {
            profile_id: profile.id.clone(),
            ..Default::default()
        };
        let outcome = match self.send(profile, secret, body, timeout).await {
            Ok(response) => receive(response, &mut metering, &mut on_text, secret).await,
            Err(error) if error.status == 400 => Outcome {
                state: "failed",
                message: Some(error.message),
                finish_reason: None,
                status: None,
                streamed: false,
                usage_observed: false,
            },
            Err(_) => Outcome::unknown("连接中断，远端是否生成或计费仍未知。"),
        };
        metering.latency_ms = timer.elapsed().as_millis() as i64;
        (outcome, metering)
    }
}

fn update_metering(value: &Value, metering: &mut Metering) {
    let usage = &value["usage"];
    let count = |value: &Value| value.as_i64().filter(|number| *number >= 0);
    if let Some(n) = count(&usage["prompt_tokens"]) {
        metering.prompt_tokens = Some(n);
    }
    if let Some(n) = count(&usage["completion_tokens"]) {
        metering.completion_tokens = Some(n);
    }
    if let Some(n) = count(&usage["prompt_tokens_details"]["cached_tokens"])
        .or_else(|| count(&usage["prompt_cache_hit_tokens"]))
    {
        metering.cached_tokens = Some(n);
    }
}

fn finish_reason(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|reason| {
            matches!(
                *reason,
                "stop" | "length" | "content_filter" | "tool_calls" | "function_call"
            )
        })
        .map(str::to_owned)
}

async fn receive(
    response: Response,
    metering: &mut Metering,
    on_text: &mut impl FnMut(&str) -> Result<()>,
    secret: Option<&str>,
) -> Outcome {
    let status = response.status().as_u16();
    let mut outcome = Outcome {
        state: "completed",
        message: None,
        finish_reason: None,
        status: Some(status),
        streamed: false,
        usage_observed: false,
    };
    if !(200..300).contains(&status) {
        outcome.state = if status >= 500 || status == 408 {
            "outcome_unknown"
        } else {
            "failed"
        };
        outcome.message = Some(format!("模型服务返回 HTTP {status}。"));
        return outcome;
    }
    let is_sse = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|kind| {
            kind.split(';')
                .next()
                .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
        });
    outcome.streamed = is_sse;
    let mut total_bytes = 0;
    let mut pending = String::new();
    let mut redact = |text: &str| -> Result<()> {
        total_bytes += text.len();
        if total_bytes > 2_000_000 {
            return Err(StudioError::new(413, "生成正文超过单次保存上限"));
        }
        if let Some(key) = secret.filter(|key| !key.is_empty()) {
            pending.push_str(text);
            if pending.contains(key) {
                pending = pending.replace(key, "[已隐去]");
            }
            let max = pending.len().min(key.len() - 1);
            let held = (1..=max)
                .rev()
                .find(|&len| pending.as_bytes().ends_with(&key.as_bytes()[..len]))
                .unwrap_or(0);
            let safe = pending.len() - held;
            if safe != 0 {
                on_text(&pending[..safe])?;
                pending.drain(..safe);
            }
            Ok(())
        } else {
            on_text(text)
        }
    };
    let received = if is_sse {
        receive_stream(response, metering, &mut redact, &mut outcome).await
    } else {
        receive_json(response, metering, &mut redact, &mut outcome).await
    };
    if let Err(message) = received {
        outcome.state = "outcome_unknown";
        outcome.message = Some(message);
    } else if !pending.is_empty() {
        if let Err(error) = on_text(&pending) {
            outcome.state = "outcome_unknown";
            outcome.message = Some(error.message);
        }
    }
    outcome.usage_observed =
        metering.prompt_tokens.is_some() && metering.completion_tokens.is_some();
    outcome
}

async fn receive_stream(
    response: Response,
    metering: &mut Metering,
    on_text: &mut impl FnMut(&str) -> Result<()>,
    outcome: &mut Outcome,
) -> std::result::Result<(), String> {
    let events = response.bytes_stream().eventsource();
    futures_util::pin_mut!(events);
    while let Some(event) = events.next().await {
        let event = event.map_err(|_| "流式连接中断，已保留收到的草稿。".to_owned())?;
        if event.data.trim() == "[DONE]" {
            return Ok(());
        }
        if event.data.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(&event.data)
            .map_err(|_| "中转服务返回了无效的流式数据。".to_owned())?;
        update_metering(&value, metering);
        if !value["error"].is_null() {
            return Err("模型报告流式错误，结果仍未知。".into());
        }
        let first = &value["choices"][0];
        if let Some(reason) = finish_reason(&first["finish_reason"]) {
            outcome.finish_reason = Some(reason);
        }
        if let Some(text) = first["delta"]["content"]
            .as_str()
            .filter(|text| !text.is_empty())
        {
            on_text(text).map_err(|error| error.message)?;
        }
    }
    if outcome.finish_reason.is_some()
        && metering.prompt_tokens.is_some()
        && metering.completion_tokens.is_some()
    {
        Ok(())
    } else {
        Err("流式连接未返回结束标记，已保留收到的草稿。".into())
    }
}

async fn receive_json(
    response: Response,
    metering: &mut Metering,
    on_text: &mut impl FnMut(&str) -> Result<()>,
    outcome: &mut Outcome,
) -> std::result::Result<(), String> {
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(part) = stream.next().await {
        bytes.extend_from_slice(&part.map_err(|_| "模型响应中断。".to_owned())?);
        if bytes.len() > 4_000_000 {
            return Err("模型响应超过大小限制。".into());
        }
    }
    let value: Value =
        serde_json::from_slice(&bytes).map_err(|_| "模型响应不是兼容的 JSON。".to_owned())?;
    update_metering(&value, metering);
    if !value["error"].is_null() {
        return Err("模型返回了错误响应。".into());
    }
    let first = &value["choices"][0];
    outcome.finish_reason = finish_reason(&first["finish_reason"]);
    if let Some(text) = first["message"]["content"].as_str() {
        on_text(text).map_err(|error| error.message)?;
    }
    Ok(())
}

pub fn probe_body_valid(body: &Value) -> bool {
    let max = body
        .get("max_tokens")
        .or_else(|| body.get("max_completion_tokens"))
        .and_then(Value::as_u64);
    let messages = body["messages"].as_array();
    max.is_some_and(|n| (1..=160).contains(&n))
        && messages.is_some_and(|list| {
            list.len() == 1
                && list[0]["role"] == json!("user")
                && list[0]["content"]
                    .as_str()
                    .is_some_and(|value| value.len() <= 1024)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profiles::Capabilities;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        thread,
    };

    fn serve(body: String, truncate: bool) -> (Profile, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
        let worker = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut header = String::new();
            let mut size = 0;
            let mut authenticated = false;
            loop {
                header.clear();
                reader.read_line(&mut header).unwrap();
                if header == "\r\n" || header.is_empty() {
                    break;
                }
                let lower = header.to_ascii_lowercase();
                if let Some(rest) = lower.strip_prefix("content-length:") {
                    size = rest.trim().parse().unwrap();
                }
                if lower == "authorization: bearer fixture-key\r\n" {
                    authenticated = true;
                }
            }
            assert!(authenticated);
            reader.read_exact(&mut vec![0; size]).unwrap();
            let length = if truncate {
                body.len() + 100
            } else {
                body.len()
            };
            write!(socket, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n{body}").unwrap();
            socket.flush().unwrap();
        });
        (
            Profile {
                id: "test".into(),
                version: "v1".into(),
                name: "fixture".into(),
                base_url,
                model: "fixture".into(),
                credential_ref: "none".into(),
                needs_key: true,
                direct: true,
                token_limit_field: "max_tokens".into(),
                capabilities: Capabilities::default(),
            },
            worker,
        )
    }

    fn exercise(body: String, truncate: bool) -> (Outcome, Metering, String) {
        let (profile, worker) = serve(body, truncate);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut text = String::new();
        let remote = Remote::new().unwrap();
        let (outcome, usage) = runtime.block_on(remote.call(
            &profile,
            Some("fixture-key"),
            &json!({
                "model":"fixture","messages":[{"role":"user","content":"续写"}],"stream":true
            }),
            Duration::from_secs(4),
            |delta| {
                text.push_str(delta);
                Ok(())
            },
        ));
        worker.join().unwrap();
        (outcome, usage, text)
    }

    #[test]
    fn sse_completion_meters_usage_and_redacts_across_chunks() {
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"林夏读到 fixture-\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"key。\"},\"finish_reason\":\"stop\"}]}\n\n",
            "data: {\"usage\":{\"prompt_tokens\":18,\"completion_tokens\":7},\"choices\":[]}\n\n",
            "data: [DONE]\n\n").to_string();
        let (outcome, metering, text) = exercise(body, false);
        assert_eq!(outcome.state, "completed");
        assert_eq!(outcome.finish_reason.as_deref(), Some("stop"));
        assert_eq!(text, "林夏读到 [已隐去]。");
        assert_eq!(metering.prompt_tokens, Some(18));
        assert_eq!(metering.cached_tokens, None);
    }

    #[test]
    fn physical_disconnect_reports_unknown_and_keeps_received_chinese() {
        let (outcome, metering, text) = exercise(
            "data: {\"choices\":[{\"delta\":{\"content\":\"她推开门\"}}]}\n\n".into(),
            true,
        );
        assert_eq!(outcome.state, "outcome_unknown");
        assert_eq!(text, "她推开门");
        assert_eq!(metering.completion_tokens, None);
    }
}
