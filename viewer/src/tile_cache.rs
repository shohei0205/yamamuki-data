use anyhow::{Context, Result, ensure};
use http::{Request, Response, StatusCode, header};
use http_cache_semantics::{AfterResponse, BeforeRequest, CacheOptions, CachePolicy};
use reqwest::blocking::Client;
use std::{
    fs::{self, File},
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    time::{Duration, SystemTime},
};

pub const MAX_TILE_BYTES: u64 = 1024 * 1024;
const MAX_POLICY_BYTES: u64 = 64 * 1024;

pub fn read_body(response: reqwest::blocking::Response) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    response.take(MAX_TILE_BYTES + 1).read_to_end(&mut body)?;
    ensure!(
        body.len() as u64 <= MAX_TILE_BYTES,
        "地図タイルが大きすぎます"
    );
    Ok(body)
}

fn read_entry(path: &Path) -> Result<(CachePolicy, Vec<u8>)> {
    let mut file = BufReader::new(File::open(path)?);
    let mut metadata = Vec::new();
    file.by_ref()
        .take(MAX_POLICY_BYTES + 1)
        .read_until(b'\n', &mut metadata)?;
    ensure!(
        metadata.last() == Some(&b'\n') && metadata.len() as u64 <= MAX_POLICY_BYTES,
        "タイルキャッシュのヘッダーが不正です"
    );
    let policy = serde_json::from_slice(&metadata)?;
    let mut body = Vec::new();
    file.take(MAX_TILE_BYTES + 1).read_to_end(&mut body)?;
    ensure!(
        body.len() as u64 <= MAX_TILE_BYTES,
        "タイルキャッシュが大きすぎます"
    );
    Ok((policy, body))
}

fn save_entry(path: &Path, policy: &CachePolicy, body: &[u8]) -> Result<()> {
    let dir = path
        .parent()
        .context("タイルキャッシュの保存先が不正です")?;
    fs::create_dir_all(dir)?;
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(&mut temp, policy)?;
    temp.write_all(b"\n")?;
    temp.write_all(body)?;
    temp.persist(path)?;
    Ok(())
}

pub fn load<T>(
    client: &Client,
    url: &str,
    path: &Path,
    user_agent: &str,
    decode: impl Fn(&[u8]) -> Result<T>,
) -> Result<Option<(T, Duration)>> {
    let mut request = Request::get(url)
        .header(header::USER_AGENT, user_agent)
        .body(())?;
    let mut fetch = client.get(url).header(header::USER_AGENT, user_agent);
    let prior = match read_entry(path) {
        Ok((policy, body)) => match policy.before_request(&request, SystemTime::now()) {
            BeforeRequest::Fresh(_) => {
                if let Ok(image) = decode(&body) {
                    return Ok(Some((image, policy.time_to_live(SystemTime::now()))));
                }
                None
            }
            BeforeRequest::Stale {
                request: updated,
                matches: true,
            } => {
                fetch = fetch.headers(updated.headers.clone());
                request = Request::from_parts(updated, ());
                Some((policy, body))
            }
            BeforeRequest::Stale { matches: false, .. } => None,
        },
        Err(_) => None,
    };

    let response = fetch.send()?;
    if response.status() == StatusCode::NOT_FOUND {
        let _ = fs::remove_file(path);
        return Ok(None);
    }
    let now = SystemTime::now();
    let mut metadata = Response::builder().status(response.status()).body(())?;
    metadata.headers_mut().clone_from(response.headers());
    if response.status() == StatusCode::OK
        && !metadata.headers().contains_key(header::CACHE_CONTROL)
        && !metadata.headers().contains_key(header::EXPIRES)
    {
        metadata.headers_mut().insert(
            header::CACHE_CONTROL,
            header::HeaderValue::from_static("max-age=604800"),
        );
    }
    let (policy, body) = match prior {
        Some((old, body)) => match old.after_response(&request, &metadata, now) {
            AfterResponse::NotModified(policy, _) => (policy, body),
            AfterResponse::Modified(policy, _) => {
                (policy, read_body(response.error_for_status()?)?)
            }
        },
        None => {
            let policy = CachePolicy::new_options(
                &request,
                &metadata,
                now,
                CacheOptions {
                    shared: false,
                    ..Default::default()
                },
            );
            (policy, read_body(response.error_for_status()?)?)
        }
    };
    let image = decode(&body).inspect_err(|_| {
        let _ = fs::remove_file(path);
    })?;
    if policy.is_storable() {
        save_entry(path, &policy, &body)?;
    } else {
        let _ = fs::remove_file(path);
    }
    Ok(Some((image, policy.time_to_live(now))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_tile_revalidates_and_fresh_tile_is_read_from_disk() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/5/1/2.png", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for i in 0..2 {
                let (mut socket, _) = listener.accept().unwrap();
                let mut request = String::new();
                let mut reader = BufReader::new(&socket);
                loop {
                    let mut line = String::new();
                    let n = reader.read_line(&mut line).unwrap();
                    if n == 0 || line == "\r\n" {
                        break;
                    }
                    request.push_str(&line);
                }
                let request = request.to_ascii_lowercase();
                assert!(request.contains("user-agent: viewer-test"));
                if i == 0 {
                    assert!(!request.contains("if-none-match:"));
                    socket.write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: 4\r\nCache-Control: max-age=0\r\nETag: \"tile\"\r\n\r\ntile").unwrap();
                } else {
                    assert!(request.contains("if-none-match: \"tile\""));
                    socket.write_all(b"HTTP/1.1 304 Not Modified\r\nConnection: close\r\nCache-Control: max-age=86400\r\nETag: \"tile\"\r\n\r\n").unwrap();
                }
            }
        });
        let cache = tempfile::tempdir().unwrap();
        let path = cache.path().join("5/1/2.tile");
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap();
        let fetch = || {
            load(&client, &url, &path, "viewer-test", |bytes| {
                Ok(bytes.to_vec())
            })
            .unwrap()
            .unwrap()
        };
        assert_eq!(fetch().0, b"tile");
        let (body, ttl) = fetch();
        assert_eq!(body, b"tile");
        assert!(ttl > Duration::from_secs(86000));
        server.join().unwrap();
        let (body, ttl) = fetch();
        assert_eq!(body, b"tile");
        assert!(ttl > Duration::from_secs(86000));
    }
}
