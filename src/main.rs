use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use matrix_appservice::{
    ApplicationService, ApplicationServiceBuilder, EventContext, State,
    exports::matrix_sdk::ruma::events::room::{
        member::{MembershipChange, StrippedRoomMemberEvent},
        message::{OriginalSyncRoomMessageEvent, RoomMessageEventContent},
    },
};

use crate::{
    command::Command,
    openai::{Config, ConversationStore},
};

mod command;
mod openai;

#[derive(Debug, Parser)]
#[command(name = "matrix-openai-bot", version, about)]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Start the appservice
    Run {
        /// Configuration file path
        #[arg(
            short,
            long,
            env = "APPSERVICE_CONFIG_PATH",
            default_value = "config.yaml",
            help = "Path to the appservice configuration YAML file."
        )]
        config: String,    
    },
    /// Generate the YAML registration file for Synapse
    Generate {
        /// Configuration file path
        #[arg(
            short,
            long,
            env = "APPSERVICE_CONFIG_PATH",
            default_value = "config.yaml",
            help = "Path to the appservice configuration YAML file."
        )]
        config: String,    
        /// Output file, or "-" for stdout
        #[arg(short, long, default_value = "-")]
        output: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    match cli.command {
        CliCommand::Run { config } => run(&config).await,
        CliCommand::Generate { config, output } => generate(&config, &output).await,
    }
}

async fn run(config_path: &str) -> anyhow::Result<()> {
    let appservice = ApplicationServiceBuilder::new()
        .configuration_file(config_path)
        .build()
        .await?;

    let config = appservice.get_user_fields::<Config>()?;
    let state = ConversationStore::new(&config.openai)?;
    let appservice = appservice.with_state(state);

    appservice.add_event_handler(on_room_member).await?;
    appservice.add_event_handler(on_room_message).await?;

    if let Err(error) = appservice.run().await {
        tracing::error!("Application service encountered an fatal error // {}", error);
        return Err(error.into());
    }

    Ok(())
}

async fn generate(config_path: &str, output: &str) -> anyhow::Result<()> {
    let appservice = ApplicationServiceBuilder::new()
        .configuration_file(config_path)
        .build()
        .await?;

    let registration = appservice.generate_registration()?;
    match output {
        "-" => println!("{registration}"),
        path => {
            std::fs::write(path, registration)?;
            tracing::info!("Registration file written to {path}");
        }
    }

    Ok(())
}

async fn on_room_member(
    event: StrippedRoomMemberEvent,
    appservice: ApplicationService<State<Arc<ConversationStore>>>,
    context: EventContext,
) -> anyhow::Result<()> {
    let user = appservice.get_bot().await?;
    if event.state_key != user.id() {
        return Ok(());
    }

    // Auto-join on room invite
    match event.membership_change(None) {
        MembershipChange::Invited => user.join_room(&context.room_id).await?,
        _ => (),
    };

    Ok(())
}

async fn on_room_message(
    event: OriginalSyncRoomMessageEvent,
    appservice: ApplicationService<State<Arc<ConversationStore>>>,
    context: EventContext,
) -> anyhow::Result<()> {
    let user = appservice.get_bot().await?;

    // Don't process if bot sent this message itself.
    if &context.sender == user.id() {
        return Ok(());
    }

    let room = appservice.get_room(&context.room_id).await.context("Room not found")?;
    let is_direct = room.is_direct().await;

    // Only respond directly to DMs. Group chats require explicitely mentioning the bot.
    if !is_direct
        && let Some(mentions) = event.content.mentions.clone()
        && !mentions.user_ids.contains(user.id())
    {
        return Ok(());
    }

    let device = user.get_device().await.context("Device not found")?;
    device.send_receipt(room.id(), &event.event_id).await?;

    // Is input an appservice command?
    if let Some(command) = Command::parse(event.content.body()) {
        match command {
            Command::Reset => appservice.state().clear(user.id(), room.id()).await,
            _ => (),
        }

        return Ok(());
    }

    device.send_typing(room.id(), true).await?;

    let conversation = appservice.state().get_conversation(&appservice, &user, &room).await?;

    if conversation.is_empty().await && is_direct {
        conversation.backfill().await?;
    }

    let response = conversation.send_prompt(event.content.body().to_string()).await?;
    let response_id = device
        .send_message(room.id(), RoomMessageEventContent::text_markdown(response))
        .await?;
    conversation.insert_dialog(event.event_id, response_id).await;

    device.send_typing(room.id(), false).await?;

    Ok(())
}
