use boj_discord_bot::{
    discord_commands::{self, Commands},
    solved_ac::SolvedAc,
};
use serenity::all::{
    ApplicationId, ChannelId, Client, Context, CreateAllowedMentions, CreateAttachment,
    CreateMessage, EditInteractionResponse, EventHandler, GatewayIntents, GuildId, Interaction,
    Ready,
};
use serenity::async_trait;
use std::{env, error::Error, io, path::PathBuf, sync::Arc};

async fn checkin_scheduler(
    commands: Arc<Commands>,
    http: Arc<serenity::http::Http>,
    channel: ChannelId,
) {
    let mut timer = tokio::time::interval(std::time::Duration::from_secs(60));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        timer.tick().await;
        let store = Arc::clone(&commands);
        let due = tokio::task::spawn_blocking(move || {
            store.claim_reminders(channel.get(), std::time::SystemTime::now().into())
        })
        .await;
        let notices = match due {
            Ok(Ok(notices)) => notices,
            _ => {
                eprintln!("학습 알림 스캔 실패. 데이터 폴더 권한을 확인하세요.");
                continue;
            }
        };
        for notice in notices {
            let store = Arc::clone(&commands);
            let refreshed = tokio::task::spawn_blocking(move || {
                let progress = store.refresh_reminder(&notice, std::time::SystemTime::now().into());
                // API calls may cross midnight or race with /skip. Check again
                // after lookup before handing the message to Discord.
                let progress = progress.and_then(|progress| {
                    if store
                        .checkins()
                        .allowed(&notice, std::time::SystemTime::now().into())?
                    {
                        Ok(progress)
                    } else {
                        Ok(None)
                    }
                });
                (notice, progress)
            })
            .await;
            let (notice, progress) = match refreshed {
                Ok((notice, Ok(Some(progress)))) => (notice, progress),
                Ok((notice, _)) => {
                    // No Discord send happened, so pending/skip/completion must
                    // not consume today's one delivery opportunity.
                    let store = Arc::clone(&commands);
                    let _ =
                        tokio::task::spawn_blocking(move || store.checkins().retry(&notice)).await;
                    continue;
                }
                _ => {
                    eprintln!("완료 체크 알림 발송 전 재확인 실패.");
                    continue;
                }
            };
            let content = if progress.verified {
                let mut lines = vec![format!(
                    "📣 <@{}> 오늘 추천 {}/{}개 해결 확인 · {}개 미완료입니다.",
                    notice.user,
                    progress.checked,
                    progress.total,
                    progress.remaining.len()
                )];
                for problem in &progress.remaining {
                    lines.push(format!("• {}\n<{}>", problem.title, problem.submission_url));
                }
                lines.push("solved.ac 조회 기준입니다. 제출 후 반영이 늦으면 나중에 /check로 확인해주세요.".into());
                lines.join("\n")
            } else {
                format!(
                    "📣 <@{}> 오늘 추천 {}개 중 {}개는 아직 **완료 체크가 없습니다** 👀\n풀었다면 `/done`, 일부만 풀었다면 `/done problem:1`로 체크해주세요. 실제 풀이 여부를 조회한 알림은 아닙니다.",
                    notice.user,
                    progress.total,
                    progress.remaining.len()
                )
            };
            let mut message = CreateMessage::new().allowed_mentions(
                CreateAllowedMentions::new()
                    .all_users(false)
                    .all_roles(false)
                    .everyone(false)
                    .users([notice.user]),
            );
            if content.encode_utf16().count() > 1900 {
                message = message.content(format!("📣 <@{}> 오늘 미완료 문제 {}개를 파일로 첨부했습니다. solved.ac 조회 기준입니다.", notice.user, progress.remaining.len())).add_file(CreateAttachment::bytes(content.into_bytes(), "remaining-problems.txt"));
            } else {
                message = message.content(content);
            }
            if let Err(error) = channel.send_message(&http, message).await {
                let rejected = matches!(&error, serenity::Error::Http(error) if error.status_code().is_some_and(|status| status.is_client_error()));
                if rejected {
                    let store = Arc::clone(&commands);
                    let retry =
                        tokio::task::spawn_blocking(move || store.checkins().retry(&notice)).await;
                    if !matches!(retry, Ok(Ok(()))) {
                        eprintln!("알림 재시도 상태 저장 실패.");
                    }
                    eprintln!(
                        "채널 알림이 Discord에서 거부됐습니다. 봇의 채널 보기·메시지 보내기 권한과 요청 제한을 확인하세요. 다음 스캔에서 재시도합니다."
                    );
                } else {
                    eprintln!(
                        "채널 알림 전송 결과가 불확실합니다. 중복 방지를 위해 오늘은 재발송하지 않습니다."
                    );
                }
            }
        }
    }
}

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
        let response = if text.encode_utf16().count() > 1900 {
            EditInteractionResponse::new()
                .content("응답이 길어 전체 내용을 파일로 첨부했습니다.")
                .new_attachment(CreateAttachment::bytes(
                    text.into_bytes(),
                    "recommendations.txt",
                ))
        } else {
            EditInteractionResponse::new().content(text)
        };
        if command.edit_response(&ctx.http, response).await.is_err() {
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
            commands: Arc::clone(&commands),
        })
        .await?;

    // Upsert our commands without deleting unrelated guild commands.
    for definition in discord_commands::definitions() {
        guild.create_command(&client.http, definition).await?;
    }
    println!("학습 설정 명령 등록 완료. /help로 사용법을 확인하세요.");
    tokio::spawn(checkin_scheduler(
        commands,
        Arc::clone(&client.http),
        channel,
    ));
    println!(
        "학습 알림 활성화: 연결 계정은 실제 풀이 조회, 미연결 계정은 직접 체크 기준 (60초 간격 확인)"
    );

    let shards = client.shard_manager.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            shards.shutdown_all().await;
        }
    });
    client.start().await?;
    Ok(())
}
