//! LLM client abstraction plus an offline echo client and an OpenAI-compatible streaming client.

use async_trait::async_trait;
use futures::stream::{self, BoxStream, StreamExt};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
}

#[async_trait]
pub trait LlmClient: Send + Sync {
    async fn stream_chat(
        &self,
        messages: &[Message],
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<String>>>;
}

/// Offline stand-in used when no API key is configured; echoes the last message back word by word.
#[derive(Default)]
pub struct EchoClient;

#[async_trait]
impl LlmClient for EchoClient {
    async fn stream_chat(
        &self,
        messages: &[Message],
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<String>>> {
        let reply = messages.last().map(|m| m.content.clone()).unwrap_or_default();
        let words: Vec<anyhow::Result<String>> = reply
            .split_whitespace()
            .map(|w| Ok(format!("{w} ")))
            .collect();
        Ok(stream::iter(words).boxed())
    }
}

/// Streams chat completions from an OpenAI-compatible `/v1/chat/completions` endpoint.
pub struct OpenAiClient {
    api_key: String,
    model: String,
    base_url: String,
    http: reqwest::Client,
}

impl OpenAiClient {
    pub fn new(api_key: String, model: String, base_url: Option<String>) -> Self {
        Self {
            api_key,
            model,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com/v1".to_string()),
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [Message],
    stream: bool,
}

#[derive(Deserialize)]
struct ChatChunk {
    choices: Vec<ChatChoice>,
}

#[derive(Deserialize)]
struct ChatChoice {
    delta: ChatDelta,
}

#[derive(Deserialize, Default)]
struct ChatDelta {
    content: Option<String>,
}

#[async_trait]
impl LlmClient for OpenAiClient {
    async fn stream_chat(
        &self,
        messages: &[Message],
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<String>>> {
        let body = ChatRequest {
            model: &self.model,
            messages,
            stream: true,
        };
        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;

        let mut byte_stream = response.bytes_stream();
        // OpenAI streams newline-delimited `data: {...}` SSE frames terminated by `data: [DONE]`.
        let stream = async_stream::stream! {
            let mut buffer = String::new();
            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(anyhow::anyhow!(e));
                        continue;
                    }
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].trim().to_string();
                    buffer.drain(..=pos);
                    let Some(data) = line.strip_prefix("data: ") else { continue };
                    if data == "[DONE]" {
                        return;
                    }
                    match serde_json::from_str::<ChatChunk>(data) {
                        Ok(parsed) => {
                            if let Some(choice) = parsed.choices.into_iter().next()
                                && let Some(content) = choice.delta.content {
                                    yield Ok(content);
                                }
                        }
                        Err(e) => yield Err(anyhow::anyhow!(e)),
                    }
                }
            }
        };
        Ok(Box::pin(stream))
    }
}

/// Streams messages from Anthropic's `/v1/messages` endpoint.
pub struct AnthropicClient {
    api_key: String,
    model: String,
    base_url: String,
    http: reqwest::Client,
}

impl AnthropicClient {
    pub fn new(api_key: String, model: String, base_url: Option<String>) -> Self {
        Self {
            api_key,
            model,
            base_url: base_url.unwrap_or_else(|| "https://api.anthropic.com/v1".to_string()),
            http: reqwest::Client::new(),
        }
    }
}

#[derive(Serialize)]
struct AnthropicRequest<'a> {
    model: &'a str,
    max_tokens: u32,
    messages: &'a [Message],
    stream: bool,
}

#[async_trait]
impl LlmClient for AnthropicClient {
    async fn stream_chat(
        &self,
        messages: &[Message],
    ) -> anyhow::Result<BoxStream<'static, anyhow::Result<String>>> {
        let body = AnthropicRequest {
            model: &self.model,
            max_tokens: 4096,
            messages,
            stream: true,
        };
        let response = self
            .http
            .post(format!("{}/messages", self.base_url))
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .json(&body)
            .send()
            .await?
            .error_for_status()?;

        let mut byte_stream = response.bytes_stream();
        // Anthropic streams `event:`/`data:` SSE pairs; only `content_block_delta` data carries text.
        let stream = async_stream::stream! {
            let mut buffer = String::new();
            while let Some(chunk) = byte_stream.next().await {
                let chunk = match chunk {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(anyhow::anyhow!(e));
                        continue;
                    }
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(pos) = buffer.find('\n') {
                    let line = buffer[..pos].trim().to_string();
                    buffer.drain(..=pos);
                    let Some(data) = line.strip_prefix("data: ") else { continue };
                    match serde_json::from_str::<serde_json::Value>(data) {
                        Ok(value) => {
                            let text = (value.get("type").and_then(|t| t.as_str()) == Some("content_block_delta"))
                                .then(|| value.get("delta").and_then(|d| d.get("text")).and_then(|t| t.as_str()))
                                .flatten();
                            if let Some(text) = text {
                                yield Ok(text.to_string());
                            }
                        }
                        Err(e) => yield Err(anyhow::anyhow!(e)),
                    }
                }
            }
        };
        Ok(Box::pin(stream))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn echo_client_streams_words_of_last_message() {
        let client = EchoClient;
        let messages = vec![Message {
            role: "user".into(),
            content: "hello there".into(),
        }];
        let mut stream = client.stream_chat(&messages).await.unwrap();
        let mut collected = String::new();
        while let Some(chunk) = stream.next().await {
            collected.push_str(&chunk.unwrap());
        }
        assert_eq!(collected, "hello there ");
    }

    #[tokio::test]
    async fn echo_client_with_no_messages_streams_nothing() {
        let client = EchoClient;
        let mut stream = client.stream_chat(&[]).await.unwrap();
        assert!(stream.next().await.is_none());
    }
}
