use matrix_appservice::{
    Device,
    exports::matrix_sdk::ruma::{
        OwnedEventId,
        events::room::message::{RoomMessageEventContent, RoomMessageEventContentWithoutRelation},
    },
};
use serde_json::json;

use crate::openai::Processed;

pub enum Command {
    Reset,
    Help,
    Version,
    Unknown(String),
}

impl Command {
    pub fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if !trimmed.starts_with('!') {
            return None;
        }

        let mut parts = trimmed[1..].splitn(2, ' ');
        let keyword = parts.next().unwrap_or_default();
        let args = parts.next().unwrap_or("");

        Some(match keyword {
            "reset" => Command::Reset,
            "help" => Command::Help,
            "version" => Command::Version,
            other => Command::Unknown(other.to_string()),
        })
    }

    pub async fn send_message(&self, device: &Device, relates_to: OwnedEventId) -> anyhow::Result<()> {
        let content = RoomMessageEventContentWithoutRelation::text_markdown(self.as_str());
        let relation_body = json!({
            "rel_type": "nl.spacebased.matrix-openai-bot.bot_response",
            "relates_to": relates_to,
        });

        // content.with_relation(Some(serde_json::from_value(relation_body)?));
        Ok(())
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Command::Reset => "",
            Command::Help => "Help text",
            Command::Version => "Matrix AI Bot, v0.10",
            Command::Unknown(_) => "Unknown command",
        }
    }

    pub fn into_processed(&self) -> Option<Processed> {
        match self {
            Command::Reset => Some(Processed::Stop),
            _ => None,
        }
    }
}
