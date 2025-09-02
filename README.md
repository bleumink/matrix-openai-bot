# Matrix OpenAI Bot
A simple Matrix bot written in Rust that uses the OpenAI API to provide conversational AI.
Built using the [Matrix Appservice](https://github.com/bleumink/matrix-appservice) library to support end-to-end encryption using modern application service authentication mechanisms.

Conversations are tracked for direct messages with the bot. In group chats the bot only reply when explicitely mentioned and does not track context.

**Note:** Arguably this is server-to-appservice encryption. After all, the prompts still get sent to OpenAI just using TLS. But the storage of your conversations is encrypted!

## :construction: Work in progress
This bot is under development. Expect additional features, documentation and cleanup in the future.

## Usage
#### Create a configuration file
Use ```example.yaml``` as a template.

#### Create an appservice registration file
The easiest way to create this is using the docker image:
```bash
docker run -v /path/to/config.yaml:/data/config.yaml ghcr.io/bleumink/matrix-openai-bot:latest generate
```

When creating the registration file manually, be sure to include the following flags in order to opt-in to the required functionality:
```yaml
de.sorunome.msc2409.push_ephemeral: true
org.matrix.msc3202: true
io.element.msc4190: true
```

#### Register the bot with Synapse
Edit your Synapse homeserver configuration to register the appservice and opt-in to several experimental features. This is using the cutting edge after all.
```yaml
app_service_config_files:
  - /path/to/registration.yaml
experimental_features:
  msc4190_enabled: true
  msc3202_device_masquerading: true
  msc2409_to_device_messages_enabled: true
  msc3202_transaction_extensions: true
```

#### Run the bot
This can be done using Docker:
```bash
docker run -v /path/to/config.yaml:/data/config.yaml ghcr.io/bleumink/matrix-openai-bot:latest
```

Or compiling from source:
```bash
git clone https://github.com/bleumink/matrix-openai-bot.git
cd matrix-openai-bot
cargo build --release

./target/release/matrix-openai-bot run --config /path/to/config.yaml
```

## License
This project is dual-licensed under the terms of the GNU Affero General Public License v3.0 (AGPL-3.0) for open source use, and a separate commercial license for proprietary or government use. Contact info@spacebased.nl for commercial licensing.