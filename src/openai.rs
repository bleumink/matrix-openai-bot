use serde::{Deserialize, Serialize};
use url::Url;

use crate::openai::tools::ToolCall;

pub use self::conversation::{ConversationStore, Processed};

mod conversation;
mod tools;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub openai: OpenAIConfig,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OpenAIConfig {
    pub endpoint: Url,
    pub api_key: String,
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIChoice {
    pub index: u16,
    pub message: OpenAIMessage,
    // pub logprobs: Option<String>,
    // pub finish_reason: String,
}

#[derive(Debug, Deserialize)]
pub struct OpenAIResponse {
    // pub id: String,
    pub object: String,
    pub created: u32,
    pub model: String,
    pub choices: Vec<OpenAIChoice>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OpenAIMessage {
    pub role: String,
    pub content: Option<MessageContent>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    // pub refusal: Option<String>,
    // pub annotations: Option<Vec<String>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OpenAIImageContent {
    #[serde(rename = "type")]
    kind: String,
    image_url: ImageUrl,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ImageUrl {
    url: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Images(Vec<OpenAIImageContent>),
}

pub enum Role {
    User,
    Assistant,
}

impl Role {
    fn as_str(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
