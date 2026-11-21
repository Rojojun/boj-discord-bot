use crate::discord_commands::Commands;
use std::{io, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    sync::Semaphore,
    time::timeout,
};
use url::Url;

/// Local HTTP listener. Publish through an HTTPS reverse proxy or SSH forwarding.
/// Bind before starting Discord so a port conflict fails startup, not account linking.
pub async fn serve(listener: TcpListener, commands: Arc<Commands>) -> io::Result<()> {
    let slots = Arc::new(Semaphore::new(4));
    loop {
        let permit = Arc::clone(&slots)
            .acquire_owned()
            .await
            .map_err(io::Error::other)?;
        let (socket, _) = listener.accept().await?;
        let commands = Arc::clone(&commands);
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(_error) = respond(socket, commands).await {
                // Never log request targets, codes, state or tokens.
                eprintln!("OAuth 콜백 처리 실패. /connect로 다시 연결하세요.");
            }
        });
    }
}

async fn respond(
    mut socket: impl AsyncRead + AsyncWrite + Unpin,
    commands: Arc<Commands>,
) -> io::Result<()> {
    let mut request = Vec::new();
    let read = timeout(Duration::from_secs(5), async {
        let mut buffer = [0u8; 1024];
        while !request.windows(4).any(|part| part == b"\r\n\r\n") {
            let count = socket.read(&mut buffer).await?;
            if count == 0 || request.len() + count > 8192 {
                return Err(io::Error::other("invalid HTTP request"));
            }
            request.extend_from_slice(&buffer[..count]);
        }
        Ok::<(), io::Error>(())
    })
    .await;
    if !matches!(read, Ok(Ok(()))) {
        return Ok(());
    }
    let input = std::str::from_utf8(&request).unwrap_or_default();
    let parsed = parse_request(input);
    let (status, body) = match parsed {
        Ok((state, code)) => {
            let success =
                tokio::task::spawn_blocking(move || commands.complete_link(&state, &code))
                    .await
                    .unwrap_or(false);
            if success {
                (
                    "200 OK",
                    "계정 연결 완료. Discord로 돌아가 /setup과 /today를 확인하세요.",
                )
            } else {
                (
                    "400 Bad Request",
                    "계정 연결 실패 또는 만료. Discord에서 /connect를 다시 실행하세요.",
                )
            }
        }
        Err(message) => ("400 Bad Request", message),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    timeout(
        Duration::from_secs(5),
        socket.write_all(response.as_bytes()),
    )
    .await??;
    Ok(())
}

fn parse_request(input: &str) -> Result<(String, String), &'static str> {
    let mut words = input.lines().next().unwrap_or_default().split_whitespace();
    if words.next() != Some("GET") {
        return Err("GET 요청만 지원합니다.");
    }
    let target = words.next().ok_or("잘못된 요청입니다.")?;
    if !target.starts_with('/') || target.starts_with("//") {
        return Err("잘못된 요청입니다.");
    }
    let url = Url::parse(&format!("http://localhost{target}")).map_err(|_| "잘못된 요청입니다.")?;
    if url.path() != "/oauth/callback" {
        return Err("콜백 경로는 /oauth/callback입니다.");
    }
    let params: Vec<_> = url.query_pairs().collect();
    if params.iter().any(|(key, _)| key == "error") {
        return Err("인증이 취소됐습니다. /connect로 다시 연결하세요.");
    }
    let one = |key| {
        let values: Vec<_> = params
            .iter()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value)
            .collect();
        if values.len() == 1 && !values[0].is_empty() {
            Some(values[0].to_string())
        } else {
            None
        }
    };
    Ok((
        one("state").ok_or("state가 없거나 중복됐습니다.")?,
        one("code").ok_or("code가 없거나 중복됐습니다.")?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn callback_accepts_exact_path_and_single_code_state_only() {
        assert_eq!(
            parse_request("GET /oauth/callback?state=abc&code=x%2By HTTP/1.1\r\n\r\n").unwrap(),
            ("abc".into(), "x+y".into())
        );
        for request in [
            "POST /oauth/callback?state=s&code=c HTTP/1.1",
            "GET /other?state=s&code=c HTTP/1.1",
            "GET /oauth/callback?state=s&code=c&code=d HTTP/1.1",
            "GET /oauth/callback?state=s&code= HTTP/1.1",
            "GET /oauth/callback?state=s&error=access_denied HTTP/1.1",
            "GET https://example.com/oauth/callback?state=s&code=c HTTP/1.1",
        ] {
            assert!(parse_request(request).is_err());
        }
    }

    #[tokio::test]
    async fn callback_rejects_unknown_state_without_echoing_secrets() {
        let data = tempfile::tempdir().unwrap();
        let commands = Arc::new(Commands::new(
            data.path(),
            Some(crate::solved_ac::SolvedAc::new(
                "app".into(),
                "secret".into(),
            )),
        ));
        let (mut client, socket) = tokio::io::duplex(16384);
        let task = tokio::spawn(async move {
            respond(socket, commands).await.unwrap();
        });
        client.write_all(b"GET /oauth/callback?state=private-state&code=private-code HTTP/1.1\r\nHost: localhost\r\n\r\n").await.unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).await.unwrap();
        task.await.unwrap();
        assert!(response.starts_with("HTTP/1.1 400"));
        assert!(response.contains("Cache-Control: no-store"));
        assert!(!response.contains("private-state"));
        assert!(!response.contains("private-code"));
        assert!(!data.path().join("users").exists());
    }
}
