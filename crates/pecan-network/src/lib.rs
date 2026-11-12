//! Bounded asynchronous networking shared by Markdown and Quick Look images.
use std::{sync::Arc, time::Duration};

use anyhow::{Context as _, Result, ensure};
use futures_lite::AsyncReadExt as _;
use http_client::{AsyncBody, HttpClient, RedirectPolicy, Request, RequestTimeout, Response, http};

const MAX_BODY_BYTES: u64 = 16 * 1024 * 1024;

/// One pooled HTTP client and one bounded network executor.
#[derive(Debug)]
pub struct Client {
    runtime: Option<tokio::runtime::Runtime>,
    executor: tokio::runtime::Handle,
    client: reqwest::Client,
    slots: Arc<async_lock::Semaphore>,
}

impl Client {
    /// Create one network worker and a pooled, bounded TLS client.
    /// # Errors
    /// Returns an error if the executor or TLS client cannot be constructed.
    pub fn new() -> Result<Self, NetworkError> {
        // Finish fallible TLS setup before owning a runtime, so an error cannot
        // invoke Tokio's blocking destructor inside the caller's executor.
        let client = reqwest::Client::builder()
            .user_agent(concat!("Pecan/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .redirect_policy(redirect_policy(10))
            .pool_max_idle_per_host(2)
            .pool_idle_timeout(Duration::from_secs(30))
            .build()
            .context("creating TLS client")
            .map_err(NetworkError::new)?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("pecan-network")
            .enable_all()
            .build()
            .context("creating network executor")
            .map_err(NetworkError::new)?;
        Ok(Self {
            executor: runtime.handle().clone(),
            runtime: Some(runtime),
            client,
            slots: Arc::new(async_lock::Semaphore::new(2)),
        })
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // A caller can release its last client inside another async executor.
        // Tokio's blocking Runtime destructor would panic in that context.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}
impl HttpClient for Client {
    fn user_agent(&self) -> Option<&http::HeaderValue> {
        None
    }
    fn proxy(&self) -> Option<&url::Url> {
        None
    }
    fn send(
        &self,
        request: Request<AsyncBody>,
    ) -> std::pin::Pin<Box<dyn Future<Output = Result<Response<AsyncBody>>> + Send>> {
        let client = self.client.clone();
        let slots = self.slots.clone();
        let job = CancelOnDrop(self.executor.spawn(async move {
            let _permit = slots.acquire().await;
            let (parts, body) = request.into_parts();
            let redirect = match parts.extensions.get::<RedirectPolicy>() {
                Some(RedirectPolicy::NoFollow) => reqwest::redirect::Policy::none(),
                Some(RedirectPolicy::FollowLimit(count)) => redirect_policy(*count as usize),
                _ => redirect_policy(10),
            };
            let timeout = parts
                .extensions
                .get::<RequestTimeout>()
                .map_or(Duration::from_secs(30), |timeout| {
                    timeout.0.min(Duration::from_secs(30))
                });
            let mut payload = Vec::new();
            body.take(MAX_BODY_BYTES + 1)
                .read_to_end(&mut payload)
                .await?;
            ensure!(
                payload.len() as u64 <= MAX_BODY_BYTES,
                "request exceeds 16 MiB"
            );
            let mut response = client
                .request(parts.method, parts.uri.to_string())
                .headers(parts.headers)
                .body(payload)
                .redirect_policy(redirect)
                .timeout(timeout)
                .send()
                .await
                .context("downloading image")?;
            ensure!(
                response
                    .content_length()
                    .is_none_or(|size| size <= MAX_BODY_BYTES),
                "image exceeds 16 MiB"
            );
            let mut result = Response::builder().status(response.status());
            *result.headers_mut().context("constructing HTTP response")? =
                response.headers().clone();
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                ensure!(
                    bytes.len().saturating_add(chunk.len()) as u64 <= MAX_BODY_BYTES,
                    "image exceeds 16 MiB"
                );
                bytes.extend_from_slice(&chunk);
            }
            Ok(result.body(bytes.into())?)
        }));
        Box::pin(async move {
            let mut job = job;
            (&mut job.0).await.context("image download task")?
        })
    }
}

/// Follow at most `limit` redirects that neither downgrade HTTPS nor leave the public internet.
///
/// Document images load without a click, so a public URL must not be able to bounce the
/// request to a router, a cloud metadata endpoint or another service on the local network.
/// Hostnames are not resolved here; only address literals and `localhost` are recognized.
fn redirect_policy(limit: usize) -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(move |attempt| {
        if attempt.previous().len() > limit {
            return attempt.error("too many redirects");
        }
        let decision = match (attempt.previous().first(), attempt.previous().last()) {
            (Some(origin), Some(previous)) => redirect_allowed(origin, previous, attempt.url()),
            _ => Ok(()),
        };
        match decision {
            Ok(()) => attempt.follow(),
            Err(reason) => attempt.error(reason),
        }
    })
}

fn redirect_allowed(
    origin: &url::Url,
    previous: &url::Url,
    target: &url::Url,
) -> Result<(), &'static str> {
    if previous.scheme() == "https" && target.scheme() != "https" {
        return Err("refusing a redirect from HTTPS to an insecure URL");
    }
    if is_internal(target) && !is_internal(origin) {
        return Err("refusing a redirect from a public host to a local network address");
    }
    Ok(())
}

fn is_internal(url: &url::Url) -> bool {
    use std::net::{Ipv4Addr, Ipv6Addr};
    fn v4(ip: Ipv4Addr) -> bool {
        let [a, b, ..] = ip.octets();
        ip.is_loopback()
            || ip.is_private()
            || ip.is_link_local()
            || ip.is_unspecified()
            || ip.is_broadcast()
            // Carrier-grade NAT, 100.64.0.0/10.
            || (a == 100 && (64..128).contains(&b))
    }
    fn v6(ip: Ipv6Addr) -> bool {
        let first = ip.segments()[0];
        ip.is_loopback()
            || ip.is_unspecified()
            || (first & 0xfe00) == 0xfc00
            || (first & 0xffc0) == 0xfe80
            || ip.to_ipv4_mapped().is_some_and(v4)
    }
    match url.host() {
        Some(url::Host::Ipv4(ip)) => v4(ip),
        Some(url::Host::Ipv6(ip)) => v6(ip),
        Some(url::Host::Domain(domain)) => {
            let domain = domain.trim_end_matches('.');
            domain.eq_ignore_ascii_case("localhost")
                || domain
                    .rsplit_once('.')
                    .is_some_and(|(_, top)| top.eq_ignore_ascii_case("localhost"))
        }
        None => true,
    }
}

/// Removing an offscreen image cancels its download instead of leaving detached work.
struct CancelOnDrop(tokio::task::JoinHandle<Result<Response<AsyncBody>>>);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A recoverable network executor or TLS configuration failure.
#[derive(Debug, thiserror::Error)]
#[error("network client failed: {source:#}")]
pub struct NetworkError {
    #[source]
    source: anyhow::Error,
}
impl NetworkError {
    fn new(source: anyhow::Error) -> Self {
        Self { source }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn client() -> Arc<dyn HttpClient> {
        Arc::new(Client::new().unwrap())
    }
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    #[test]
    fn client_can_be_released_inside_another_async_executor() {
        let outer = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        outer.block_on(async {
            let client = Client::new().unwrap();
            drop(client);
        });
    }

    #[test]
    fn redirects_cannot_downgrade_or_reach_the_local_network_from_public_hosts() {
        let url = |text: &str| url::Url::parse(text).unwrap();
        let public = url("https://example.com/a.png");
        for target in [
            "http://example.com/a.png",
            "https://127.0.0.1/a.png",
            "https://169.254.169.254/latest/meta-data",
            "https://10.0.0.1/a.png",
            "https://192.168.1.1/a.png",
            "https://[::1]/a.png",
            "https://[fd00::1]/a.png",
            "https://[::ffff:127.0.0.1]/a.png",
            "https://localhost/a.png",
            "https://printer.localhost/a.png",
        ] {
            assert!(
                redirect_allowed(&public, &public, &url(target)).is_err(),
                "{target}"
            );
        }
        assert!(redirect_allowed(&public, &public, &url("https://cdn.example.net/a.png")).is_ok());
        let plain = url("http://example.com/a.png");
        assert!(redirect_allowed(&plain, &plain, &url("https://example.com/a.png")).is_ok());
        let local = url("http://127.0.0.1:8080/a.png");
        assert!(redirect_allowed(&local, &local, &url("http://localhost:8080/b.png")).is_ok());
    }

    #[test]
    fn follows_redirects_and_limits_downloads() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/redirect", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for response in [
                "HTTP/1.1 302 Found\r\nLocation: /image\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                "HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nPNG",
                "HTTP/1.1 200 OK\r\nContent-Length: 16777217\r\nConnection: close\r\n\r\n",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = [0; 4096];
                assert!(stream.read(&mut request).unwrap() > 0);
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        let client = client();
        futures_lite::future::block_on(async {
            let mut response = client.get(&url, ().into(), true).await.unwrap();
            let mut bytes = Vec::new();
            response.body_mut().read_to_end(&mut bytes).await.unwrap();
            assert_eq!(bytes, b"PNG");
            assert!(
                client
                    .get(&url.replace("redirect", "large"), ().into(), true)
                    .await
                    .err()
                    .expect("oversized image must fail")
                    .to_string()
                    .contains("16 MiB")
            );
        });
        server.join().unwrap();
    }
}
