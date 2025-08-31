use std::str::FromStr;

use anyhow::Context;
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use crate::openai::{ImageUrl, MessageContent, OpenAIImageContent, OpenAIMessage};

#[derive(Debug, Serialize, Deserialize)]
pub struct ToolCall {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    function: FunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

pub enum AssistantAction {
    Reply(String),
    ToolCall(Tool),
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "name", content = "arguments")]
pub enum Tool {
    #[serde(rename = "fetch_url")]
    /// Fetch the contents of a URL and return HTML or image metadata.
    FetchUrl { url: String },
}

impl TryFrom<&ToolCall> for Tool {
    type Error = anyhow::Error;

    fn try_from(tool_call: &ToolCall) -> Result<Self, Self::Error> {
        let arguments: Value = serde_json::from_str(&tool_call.function.arguments)?;
        let tagged = json!({
            "name": tool_call.function.name,
            "arguments": arguments
        });

        Ok(serde_json::from_value(tagged)?)
    }
}

impl Tool {
    pub async fn run(&self) -> anyhow::Result<OpenAIMessage> {
        match self {
            Tool::FetchUrl { url } => fetch_url(Url::from_str(url)?).await,
        }
    }

    pub fn schemas() -> anyhow::Result<Vec<serde_json::Value>> {
        let schema = schema_for!(Tool);
        let schema_value = serde_json::to_value(&schema)?;

        let one_of = schema_value
            .get("oneOf")
            .and_then(|v| v.as_array())
            .context("Missing or invalid 'oneOf' in Tool schema")?;

        let mut tools = Vec::with_capacity(one_of.len());
        for variant in one_of {
            let props = variant
                .get("properties")
                .context("Variant missing 'properties' field")?;

            let name = props
                .get("name")
                .and_then(|v| v.get("const"))
                .and_then(|v| v.as_str())
                .context("Variant 'name.const' missing or not a string")?
                .to_string();

            let description = variant
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            let parameters = props.get("arguments").context("Variant missing 'arguments' field")?;

            tools.push(json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": description,
                    "parameters": parameters
                }
            }));
        }

        Ok(tools)
    }
}

async fn fetch_url(url: Url) -> anyhow::Result<OpenAIMessage> {
    let content = OpenAIImageContent {
        kind: "image_url".to_string(),
        image_url: ImageUrl { url: url.to_string() },
    };

    let message = OpenAIMessage {
        role: "user".to_string(),
        content: Some(MessageContent::Images(vec![content])),
        tool_calls: Vec::new(),
    };

    Ok(message)
}
