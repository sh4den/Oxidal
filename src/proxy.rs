use std::time::Duration;

use secrecy::{ExposeSecret as _, SecretString};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_socks::tcp::Socks5Stream;

pub trait Stream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin> Stream for T {}

#[derive(Clone, Default)]
pub struct ProxyConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: SecretString,
}

pub async fn open_stream(
    host: &str,
    port: u16,
    proxy: Option<&ProxyConfig>,
    timeout: Duration,
) -> anyhow::Result<Box<dyn Stream>> {
    let Some(proxy) = proxy else {
        let stream = match tokio::time::timeout(timeout, TcpStream::connect((host, port))).await {
            Ok(Ok(stream)) => stream,
            Ok(Err(e)) => anyhow::bail!("Could not reach {host}:{port}: {e}"),
            Err(_) => anyhow::bail!("Timed out connecting to {host}:{port}"),
        };
        let _ = stream.set_nodelay(true);
        return Ok(Box::new(stream));
    };

    let proxy_host = proxy.host.as_str();
    let proxy_port = proxy.port;
    let socket = match tokio::time::timeout(timeout, TcpStream::connect((proxy_host, proxy_port)))
        .await
    {
        Ok(Ok(socket)) => socket,
        Ok(Err(e)) => anyhow::bail!("Could not reach SOCKS5 proxy {proxy_host}:{proxy_port}: {e}"),
        Err(_) => anyhow::bail!("Timed out connecting to SOCKS5 proxy {proxy_host}:{proxy_port}"),
    };
    let _ = socket.set_nodelay(true);

    let username = proxy.username.trim();
    let handshake = async {
        if username.is_empty() {
            Socks5Stream::connect_with_socket(socket, (host, port)).await
        } else {
            Socks5Stream::connect_with_password_and_socket(
                socket,
                (host, port),
                username,
                proxy.password.expose_secret(),
            )
            .await
        }
    };
    match tokio::time::timeout(timeout, handshake).await {
        Ok(Ok(stream)) => Ok(Box::new(stream)),
        Ok(Err(e)) => anyhow::bail!(
            "SOCKS5 proxy {proxy_host}:{proxy_port} could not open a tunnel to {host}:{port}: {e}"
        ),
        Err(_) => {
            anyhow::bail!("Timed out during the SOCKS5 handshake with {proxy_host}:{proxy_port}")
        }
    }
}

#[cfg(test)]
#[path = "../tests/proxy.rs"]
mod tests;
