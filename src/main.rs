use boj_discord_bot::{
    agy::AgyClient,
    discord_commands::{self, Commands},
    settings::DeliveryMode,
    solved_ac::SolvedAc,
};
use serenity::all::{
    ApplicationId, ChannelId, Client, Context, CreateAllowedMentions, CreateAttachment,
    CreateMessage, EditInteractionResponse, EventHandler, GatewayIntents, GuildId, Interaction,
    Ready, UserId,
};
use serenity::async_trait;
use std::{env, error::Error, io, path::PathBuf, sync::Arc};

// Err(true) means Discord definitively rejected the message, so retry is safe.
// Never turn a private DM into a public channel message.
async fn deliver(
    commands: &Commands,
    http: &serenity::http::Http,
    channel: ChannelId,
    user: u64,
    message: CreateMessage,
) -> Result<(), bool> {
    let target = match commands.delivery(user).map_err(|_| true)? {
        DeliveryMode::Channel => channel,
        DeliveryMode::Dm => {
            UserId::new(user)
                .create_dm_channel(http)
                .await
                .map_err(|_| true)?
                .id
        }
    };
    target.send_message(http,message).await.map(|_|()).map_err(|error|matches!(&error,serenity::Error::Http(error) if error.status_code().is_some_and(|status|status.is_client_error())))
}
fn plain_message(content: String, user: u64) -> CreateMessage {
    CreateMessage::new().content(content).allowed_mentions(
        CreateAllowedMentions::new()
            .all_users(false)
            .all_roles(false)
            .everyone(false)
            .users([user]),
    )
}

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
        match tokio::task::spawn_blocking(move || {
            store.claim_resume_prompts(std::time::SystemTime::now().into())
        })
        .await
        {
            Ok(Ok(prompts)) => {
                for (user, expiry) in prompts {
                    if !commands
                        .pause()
                        .prompt_current(user, expiry, std::time::SystemTime::now().into())
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    let message = plain_message(
                        format!(
                            "<@{user}> 쉬기로 한 기간이 끝났습니다. 다시 시작하려면 `/resume`을 실행하세요. 직접 재개하기 전에는 추천과 독촉을 보내지 않습니다."
                        ),
                        user,
                    );
                    if let Err(true) = deliver(&commands, &http, channel, user, message).await {
                        let _ = commands.pause().retry_prompt(user, expiry);
                    }
                }
            }
            _ => eprintln!("학습 재개 안내 스캔 실패."),
        }
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
            if let Err(rejected) = deliver(&commands, &http, channel, notice.user, message).await {
                if rejected {
                    let store = Arc::clone(&commands);
                    let retry =
                        tokio::task::spawn_blocking(move || store.checkins().retry(&notice)).await;
                    if !matches!(retry, Ok(Ok(()))) {
                        eprintln!("알림 재시도 상태 저장 실패.");
                    }
                    eprintln!(
                        "알림이 거부됐습니다. 선택한 채널 또는 DM 권한을 확인하세요. 공개 채널로 전환하지 않습니다."
                    );
                } else {
                    eprintln!(
                        "알림 전송 결과가 불확실합니다. 중복 방지를 위해 오늘은 재발송하지 않습니다."
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
        let text = if name == "notify-test" {
            let message = plain_message(
                format!(
                    "<@{user}> 알림 수신 테스트입니다. 오늘 추천·완료 체크·알림 발송 기록은 변경하지 않았습니다."
                ),
                user,
            );
            match deliver(&commands,&ctx.http,self.channel,user,message).await {
                Ok(())=>"선택한 알림 방식으로 테스트를 보냈습니다.".into(),
                Err(_)=>"테스트 전송 실패. /setup 설정과 /settings delivery를 확인하세요. DM이면 서버 멤버의 개인 메시지 허용 여부를 확인하세요. 공개 채널로 전환하지 않았습니다.".into(),
            }
        } else {
            tokio::task::spawn_blocking(move || {
                commands.execute(user, &name, &options, std::time::SystemTime::now().into())
            })
            .await
            .unwrap_or_else(|_| "명령 처리 중 오류가 발생했습니다. 잠시 후 다시 시도하세요.".into())
        };
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
        let response = response.allowed_mentions(
            CreateAllowedMentions::new()
                .all_users(false)
                .all_roles(false)
                .everyone(false),
        );
        if command.edit_response(&ctx.http, response).await.is_err() {
            eprintln!("명령 응답 전송 실패. Discord 연결 상태를 확인하세요.");
        }
    }
}

fn command_shape(value: serde_json::Value) -> serde_json::Value {
    fn clean(value: &mut serde_json::Value) {
        match value {
            serde_json::Value::Object(fields) => {
                fields.retain(|_, value| {
                    !value.is_null() && !value.as_object().is_some_and(|map| map.is_empty())
                });
                for value in fields.values_mut() {
                    clean(value)
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    clean(value)
                }
            }
            _ => {}
        }
    }
    let mut shape = serde_json::json!({"name":value["name"],"description":value["description"],"options":value["options"],"nsfw":value["nsfw"]});
    clean(&mut shape);
    shape
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
    let ai_socket = env::var("AGY_SOCKET").unwrap_or_else(|_| "/run/boj-ai/worker.sock".into());
    if !ai_socket.trim().is_empty() {
        commands = commands.with_agy(AgyClient::new(PathBuf::from(ai_socket)));
    }
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

    // Compare definitions first: unchanged restarts require only one list request.
    // Preserve other guild commands owned by this application.
    let registered = guild.get_commands(&client.http).await?;
    for definition in discord_commands::definitions() {
        let desired = serde_json::to_value(&definition)?;
        let unchanged = registered.iter().any(|command| {
            command.kind == serenity::all::CommandType::ChatInput
                && command.name == desired["name"].as_str().unwrap_or_default()
                && serde_json::to_value(command)
                    .is_ok_and(|actual| command_shape(actual) == command_shape(desired.clone()))
        });
        if !unchanged {
            guild.create_command(&client.http, definition).await?;
        }
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

#[cfg(test)]
mod registration_tests {
    use super::command_shape;
    #[test]
    fn ignores_discord_metadata_but_detects_changed_choices_and_removed_options() {
        let desired = serde_json::json!({"name":"settings","description":"settings","options":[{"name":"delivery","type":3,"choices":[{"name":"DM","value":"dm"}]}],"nsfw":false,"name_localizations":{}});
        let mut actual = desired.clone();
        actual["id"] = serde_json::json!(123);
        actual["version"] = serde_json::json!(456);
        actual["name_localizations"] = serde_json::Value::Null;
        assert_eq!(
            command_shape(actual.clone()),
            command_shape(desired.clone())
        );
        actual["options"][0]["choices"][0]["value"] = serde_json::json!("channel");
        assert_ne!(
            command_shape(actual.clone()),
            command_shape(desired.clone())
        );
        actual["options"] = serde_json::json!([]);
        assert_ne!(command_shape(actual), command_shape(desired));
    }
}
