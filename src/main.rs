use boj_discord_bot::{
    discord_commands::{self, Commands},
    solved_ac::SolvedAc,
};
use serenity::all::{
    ApplicationId, ChannelId, Client, Context, EditInteractionResponse, EventHandler,
    GatewayIntents, GuildId, Interaction, Ready,
};
use serenity::async_trait;
use std::{env, error::Error, io, path::PathBuf, sync::Arc};

struct Handler {
    guild: GuildId,
    channel: ChannelId,
    commands: Arc<Commands>,
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
        if command.guild_id != Some(self.guild) || command.channel_id != self.channel {
            return;
        }
        if command.defer_ephemeral(&ctx.http).await.is_err() {
            eprintln!("명령 접수 실패. Discord 연결 상태를 확인하세요.");
            return;
        }
        let commands = Arc::clone(&self.commands);
        let name = command.data.name.clone();
        let options = command.data.options.clone();
        let user = command.user.id.get();
        let text = tokio::task::spawn_blocking(move || {
            commands.execute(user, &name, &options, std::time::SystemTime::now().into())
        })
        .await
        .unwrap_or_else(|_| "명령 처리 중 오류가 발생했습니다. 잠시 후 다시 시도하세요.".into());
        // Keep responses within Discord's content limit, including API-supplied handles.
        let text: String = text.chars().take(1950).collect();
        if command
            .edit_response(&ctx.http, EditInteractionResponse::new().content(text))
            .await
            .is_err()
        {
            eprintln!("명령 응답 전송 실패. Discord 연결 상태를 확인하세요.");
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
    let api = match (
        required("SOLVED_AC_CLIENT_ID"),
        required("SOLVED_AC_CLIENT_SECRET"),
    ) {
        (Ok(id), Ok(secret)) => Some(SolvedAc::new(id, secret)),
        (Err(_), Err(_)) => None,
        _ => {
            return Err(
                io::Error::other("solved.ac Client ID와 Client Secret을 모두 설정하세요.").into(),
            );
        }
    };
    let data = PathBuf::from(env::var("BOT_DATA_DIR").unwrap_or_else(|_| "data".into()));
    std::fs::create_dir_all(&data)?;
    let mut commands = Commands::new(&data, api);
    let redirect = env::var("SOLVED_AC_REDIRECT_URI")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let callback = if let Some(redirect) = redirect {
        commands = commands.with_redirect(redirect)?;
        Some(tokio::net::TcpListener::bind("127.0.0.1:8787").await?)
    } else {
        None
    };
    let commands = Arc::new(commands);
    if let Some(listener) = callback {
        let commands = Arc::clone(&commands);
        tokio::spawn(async move {
            if boj_discord_bot::oauth_callback::serve(listener, commands)
                .await
                .is_err()
            {
                eprintln!("OAuth 콜백 서버가 중단되었습니다. 봇을 재시작해주세요.");
            }
        });
        println!("OAuth 콜백 수신 준비: 127.0.0.1:8787/oauth/callback");
    }
    let mut client = Client::builder(token, GatewayIntents::empty())
        .application_id(application)
        .event_handler(Handler {
            guild,
            channel,
            commands,
        })
        .await?;

    // Upsert our commands without deleting unrelated guild commands.
    for definition in discord_commands::definitions() {
        guild.create_command(&client.http, definition).await?;
    }
    println!("학습 설정 명령 등록 완료. /help로 사용법을 확인하세요.");

    let shards = client.shard_manager.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            shards.shutdown_all().await;
        }
    });
    client.start().await?;
    Ok(())
}
