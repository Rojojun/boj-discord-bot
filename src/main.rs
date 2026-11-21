use serenity::all::{
    ApplicationId, ChannelId, Client, Context, CreateCommand, CreateInteractionResponse,
    CreateInteractionResponseMessage, EventHandler, GatewayIntents, GuildId, Interaction, Ready,
};
use serenity::async_trait;
use std::{env, error::Error, io};

struct Handler {
    guild: GuildId,
    channel: ChannelId,
}

#[async_trait]
impl EventHandler for Handler {
    async fn ready(&self, _: Context, ready: Ready) {
        println!("{} 연결 완료. 테스트 서버: {}", ready.user.name, self.guild);
    }

    async fn interaction_create(&self, ctx: Context, interaction: Interaction) {
        let Interaction::Command(command) = interaction else {
            return;
        };
        if command.guild_id != Some(self.guild)
            || command.channel_id != self.channel
            || command.data.name != "ping"
        {
            return;
        }
        let response = CreateInteractionResponse::Message(
            CreateInteractionResponseMessage::new()
                .content("퐁! 코테-마타-봇이 연결되어 있어요.")
                .ephemeral(true),
        );
        if command.create_response(&ctx.http, response).await.is_err() {
            eprintln!("/ping 응답 전송 실패. Discord 연결 상태를 확인하세요.");
        }
    }
}

fn required(name: &str) -> Result<String, io::Error> {
    env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| value.trim().to_owned())
        .ok_or_else(|| io::Error::other(format!("{name} 값을 .env 또는 환경변수에 설정하세요.")))
}

fn required_id(name: &str) -> Result<u64, io::Error> {
    required(name)?
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| io::Error::other(format!("{name}에는 올바른 Discord ID가 필요합니다.")))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    match dotenvy::dotenv() {
        Ok(_) => {}
        Err(dotenvy::Error::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(
                io::Error::other(".env 파일을 읽을 수 없습니다. 형식을 확인하세요.").into(),
            );
        }
    }
    let token = required("DISCORD_BOT_TOKEN")?;
    let application = ApplicationId::new(required_id("DISCORD_APPLICATION_ID")?);
    let guild = GuildId::new(required_id("DISCORD_GUILD_ID")?);
    let channel = ChannelId::new(required_id("DISCORD_CHANNEL_ID")?);
    let mut client = Client::builder(token, GatewayIntents::empty())
        .application_id(application)
        .event_handler(Handler { guild, channel })
        .await?;

    // Upsert just this command, preserving any other commands in the server.
    guild
        .create_command(
            &client.http,
            CreateCommand::new("ping").description("코테-마타-봇 연결 상태를 확인합니다"),
        )
        .await?;
    println!("테스트 서버에 /ping 등록 완료. 봇을 연결합니다.");

    let shards = client.shard_manager.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            shards.shutdown_all().await;
        }
    });
    client.start().await?;
    Ok(())
}
