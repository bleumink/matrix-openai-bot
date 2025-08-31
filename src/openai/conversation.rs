use std::{borrow::Cow, collections::HashMap, sync::Arc};

use anyhow::Context;
use futures::{future, StreamExt, TryStreamExt};
use matrix_appservice::{
    ApplicationService, Device, Direction, Room, State, User,
    exports::matrix_sdk::ruma::{
        OwnedEventId, OwnedRoomId, OwnedUserId, RoomId, UserId,
        events::{
            AnySyncTimelineEvent,
            room::{
                member::{MembershipChange, StrippedRoomMemberEvent},
                message::OriginalSyncRoomMessageEvent,
            },
        },
        serde::Raw,
    },
};
use reqwest::{
    Client,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{Mutex, RwLock};

use crate::{command::Command, openai::{
    tools::{AssistantAction, Tool}, Config, MessageContent, OpenAIConfig, OpenAIMessage, OpenAIResponse, Role
}};

#[derive(Debug)]

pub enum Processed {
    Continue(OwnedEventId, OpenAIMessage),
    Stop,
}

pub struct ConversationStore {
    inner: RwLock<HashMap<OwnedUserId, HashMap<OwnedRoomId, Vec<OwnedEventId>>>>,
    client: reqwest::Client,
}
#[derive(Deserialize)]
struct ExtractType<'a> {
    #[serde(borrow, rename = "type")]
    event_type: Cow<'a, str>,
}

impl ConversationStore {
    pub fn new(config: &OpenAIConfig) -> anyhow::Result<Arc<Self>> {
        let token = format!("Bearer {}", &config.api_key);
        let mut headers = HeaderMap::new();
        let mut token = HeaderValue::from_str(&token)?;
        token.set_sensitive(true);
        headers.insert(AUTHORIZATION, token);

        let client = Client::builder().use_rustls_tls().default_headers(headers).build()?;

        Ok(Arc::new(Self {
            inner: RwLock::new(HashMap::new()),
            client,
        }))
    }

    pub async fn clear(&self, user_id: &UserId, room_id: &RoomId) {
        let mut lock = self.inner.write().await;
        lock.entry(user_id.to_owned())
            .or_default()
            .entry(room_id.to_owned())
            .or_default()
            .clear();
    }

    pub async fn insert_events(
        &self,
        user_id: &UserId,
        room_id: &RoomId,
        event_ids: impl IntoIterator<Item = OwnedEventId>,
    ) {
        let mut lock = self.inner.write().await;
        lock.entry(user_id.to_owned())
            .or_default()
            .entry(room_id.to_owned())
            .or_default()
            .extend(event_ids);
    }

    pub async fn set(&self, user_id: &UserId, room_id: &RoomId, event_ids: Vec<OwnedEventId>) {
        let mut lock = self.inner.write().await;
        lock.entry(user_id.to_owned())
            .or_default()
            .entry(room_id.to_owned())
            .insert_entry(event_ids);
    }

    pub async fn get_conversation<'a>(
        &self,
        appservice: &'a ApplicationService<State<Arc<ConversationStore>>>,
        user: &'a Arc<User>,
        room: &'a Arc<Room>,
    ) -> anyhow::Result<Conversation<'a>> {
        let event_ids = {
            let mut lock = self.inner.write().await;
            lock.entry(user.id().to_owned())
                .or_default()
                .entry(room.id().to_owned())
                .or_insert_with(|| Vec::new())
                .clone()
        };

        let device = user.get_device().await.context("Device not found")?;
        let events = futures::stream::iter(event_ids)
            .map(|event_id| {
                let device = Arc::clone(&device);                
                async move {
                    let raw_event = room.get_raw_event(&event_id).await?;
                    let extracted = raw_event.deserialize_as::<ExtractType<'_>>()?;
                    match extracted.event_type.as_ref() {
                        "m.room.message" => Ok(raw_event.deserialize_as::<OriginalSyncRoomMessageEvent>()?),
                        "m.room.encrypted" => {
                            let decrypted = device.decrypt_event(raw_event.cast(), room.id()).await?;
                            Ok(decrypted.event.deserialize_as::<OriginalSyncRoomMessageEvent>()?)
                        }
                        _ => Err(anyhow::anyhow!("Invalid event type provided")),
                    }
                }
            })
            .buffered(3)
            .try_collect::<Vec<_>>()
            .await?;

        Ok(Conversation::from_events(appservice, user, room, device, &events)?)
    }
}

pub struct Conversation<'a> {
    appservice: &'a ApplicationService<State<Arc<ConversationStore>>>,
    config: OpenAIConfig,
    user: &'a User,    
    room: &'a Room,
    device: Arc<Device>,
    messages: Mutex<Vec<OpenAIMessage>>,
}

impl Conversation<'_> {
    pub fn from_events<'a>(
        appservice: &'a ApplicationService<State<Arc<ConversationStore>>>,
        user: &'a User,
        room: &'a Room,
        device: Arc<Device>,
        events: &[OriginalSyncRoomMessageEvent],
    ) -> anyhow::Result<Conversation<'a>> {
        let messages = events
            .iter()
            .map(|event| create_message(user.id(), event))
            .collect();

        let config = appservice.get_user_fields::<Config>()?.openai;
        let conversation = Conversation {
            appservice,
            config,
            user,
            room,
            device,
            messages: Mutex::new(messages),
        };

        Ok(conversation)
    }

    fn client(&self) -> &Client {
        &self.appservice.state().client
    }

    pub async fn is_empty(&self) -> bool {
        self.messages.lock().await.is_empty()
    }

    pub async fn backfill(&self) -> anyhow::Result<()> {
        let (event_ids, mut messages): (Vec<_>, Vec<_>) = self
            .room
            .get_raw_message_stream(Direction::Backward)
            .then(|raw| async { self.process_raw_event(raw?).await })
            .try_filter_map(|maybe| future::ready(Ok(maybe)) )
            .scan((), |_, result| 
                future::ready(match result {
                    Ok(Processed::Continue(id, message)) => Some((id, message)),                    
                    Ok(Processed::Stop) | Err(_) => None,                              
            }))
            .collect::<Vec<_>>()
            .await
            .into_iter()
            .rev()
            .unzip();

        let store = Arc::clone(self.appservice.state());
        store.set(self.user.id(), self.room.id(), event_ids).await;

        let mut lock = self.messages.lock().await;
        messages.append(&mut *lock);
        *lock = messages;

        Ok(())
    }

    pub async fn send_prompt(&self, prompt: String) -> anyhow::Result<String> {
        let mut messages = self.messages.lock().await;
        messages.push(OpenAIMessage {
            role: "user".to_string(),
            content: Some(MessageContent::Text(prompt)),
            tool_calls: Vec::new(),
        });
        
        let body = self.create_prompt_body(&messages)?;
        let request = self
            .client()
            .post(self.config.endpoint.clone())
            .json(&body)
            .send()
            .await?;

        let response: OpenAIResponse = request.json().await?;
        let action = into_actions(&response.choices.first().unwrap().message)?;

        if let Some(action) = action.first() {
            match action {
                AssistantAction::Reply(message) => return Ok(message.to_owned()),
                AssistantAction::ToolCall(_) => {
                    // let message = tool.run().await?;
                    // messages.push(message);
                    // // make_openai_request(messages, config).await
                }
            }
        }

        Err(anyhow::anyhow!("Unable to parse message"))
    }

    pub async fn insert_dialog(&self, prompt_id: OwnedEventId, response_id: OwnedEventId) {
        self.appservice
            .state()
            .insert_events(self.user.id(), self.room.id(), [prompt_id, response_id])
            .await
    }

    fn create_prompt_body(&self, messages: &[OpenAIMessage]) -> anyhow::Result<Value> {
        Ok(json!({
            "model": &self.config.model,
            "messages": messages,
            "tools": Tool::schemas()?,
        }))
    }

    async fn process_raw_event(&self, raw_event: Raw<AnySyncTimelineEvent>) -> anyhow::Result<Option<Processed>> {
        fn handle_event(user_id: &UserId, event: OriginalSyncRoomMessageEvent) -> anyhow::Result<Option<Processed>> {                    
            if let Some(command) = Command::parse(event.content.body()) {
                return Ok(command.into_processed())
            }
            
            let message = create_message(user_id, &event);
            Ok(Some(Processed::Continue(event.event_id, message)))
        }

        let extracted = match raw_event.deserialize_as::<ExtractType<'_>>() {
            Ok(extracted_type) => extracted_type,
            Err(error) => {
                tracing::warn!("Deserialization failed, skipping event: {error}");
                return Ok(None);
            }
        };
        
        match extracted.event_type.as_ref() {
            "m.room.member" => {
                let event = raw_event.deserialize_as::<StrippedRoomMemberEvent>()?;
                if matches!(event.membership_change(None), MembershipChange::Left) {
                    return Ok(Some(Processed::Stop));
                }
                Ok(None)
            }
            "m.room.message" => {
                let event = raw_event.deserialize_as::<OriginalSyncRoomMessageEvent>()?;
                handle_event(self.user.id(), event)
            }
            "m.room.encrypted" => {
                let decrypted = self.device.decrypt_event(raw_event.cast(), self.room.id()).await?;
                let event = decrypted.event.deserialize_as::<OriginalSyncRoomMessageEvent>()?;
                handle_event(self.user.id(), event)
            }
            _ => return Ok(None),
        }
    }
}

fn create_message(bot_id: &UserId, event: &OriginalSyncRoomMessageEvent) -> OpenAIMessage {
    let role = if event.sender == bot_id {
        Role::Assistant
    } else {
        Role::User
    };

    let message = OpenAIMessage {
        role: role.to_string(),
        content: Some(MessageContent::Text(event.content.body().to_string())),
        tool_calls: Vec::new(),
    };

    message
}

pub fn into_actions(message: &OpenAIMessage) -> anyhow::Result<Vec<AssistantAction>> {
    let mut actions = Vec::new();

    match &message.content {
        Some(MessageContent::Text(body)) => actions.push(AssistantAction::Reply(body.clone())),
        _ => return Err(anyhow::anyhow!("unknown type")),
    }

    for tool_call in &message.tool_calls {
        let tool = tool_call.try_into()?;
        actions.push(AssistantAction::ToolCall(tool));
    }

    Ok(actions)
}
