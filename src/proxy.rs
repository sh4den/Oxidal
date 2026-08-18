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
mod tests {
    use super::*;

    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    const CONNECT_OK: [u8; 10] = [5, 0, 0, 1, 0, 0, 0, 0, 0, 0];

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    fn config(addr: std::net::SocketAddr, username: &str, password: &str) -> ProxyConfig {
        ProxyConfig {
            host: addr.ip().to_string(),
            port: addr.port(),
            username: username.to_string(),
            password: SecretString::from(password),
        }
    }

    fn read_exact(socket: &mut std::net::TcpStream, len: usize) -> Vec<u8> {
        let mut buf = vec![0u8; len];
        socket.read_exact(&mut buf).expect("read");
        buf
    }

    fn read_connect_request(socket: &mut std::net::TcpStream) -> (String, u16) {
        let head = read_exact(socket, 5);
        assert_eq!(
            &head[..4],
            &[5, 1, 0, 3],
            "the target should be requested as a domain CONNECT, got {head:?}"
        );
        let domain = read_exact(socket, head[4] as usize);
        let port = read_exact(socket, 2);
        (
            String::from_utf8(domain).expect("domain"),
            u16::from_be_bytes([port[0], port[1]]),
        )
    }

    #[test]
    fn a_socks5_proxy_carries_the_connection() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            assert_eq!(read_exact(&mut socket, 3), [5, 1, 0]);
            socket.write_all(&[5, 0]).expect("method");
            let target = read_connect_request(&mut socket);
            socket.write_all(&CONNECT_OK).expect("reply");
            socket.write_all(b"welcome").expect("banner");
            (target, read_exact(&mut socket, 4))
        });

        let config = config(addr, "", "");
        let banner = runtime().block_on(async {
            let mut stream = open_stream(
                "target.example",
                2323,
                Some(&config),
                Duration::from_secs(5),
            )
            .await
            .expect("tunnel");
            stream.write_all(b"ping").await.expect("send");
            let mut banner = [0u8; 7];
            stream.read_exact(&mut banner).await.expect("banner");
            banner
        });

        let (target, relayed) = server.join().expect("server thread");
        assert_eq!(&banner, b"welcome", "data from the target should flow back");
        assert_eq!(target, ("target.example".to_string(), 2323));
        assert_eq!(relayed, b"ping", "data to the target should flow through");
    }

    #[test]
    fn proxy_credentials_are_presented_when_configured() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            assert_eq!(read_exact(&mut socket, 4), [5, 2, 0, 2]);
            socket.write_all(&[5, 2]).expect("method");
            let head = read_exact(&mut socket, 2);
            assert_eq!(head[0], 1, "password auth should use subnegotiation v1");
            let username = read_exact(&mut socket, head[1] as usize);
            let password_len = read_exact(&mut socket, 1)[0] as usize;
            let password = read_exact(&mut socket, password_len);
            socket.write_all(&[1, 0]).expect("auth ok");
            let target = read_connect_request(&mut socket);
            socket.write_all(&CONNECT_OK).expect("reply");
            (username, password, target)
        });

        let config = config(addr, "scout", "hunter2");
        runtime().block_on(async {
            open_stream("target.example", 22, Some(&config), Duration::from_secs(5))
                .await
                .expect("tunnel");
        });

        let (username, password, target) = server.join().expect("server thread");
        assert_eq!(username, b"scout");
        assert_eq!(password, b"hunter2");
        assert_eq!(target, ("target.example".to_string(), 22));
    }

    #[test]
    fn a_proxy_refusal_names_the_proxy_and_the_target() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr");

        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().expect("accept");
            read_exact(&mut socket, 3);
            socket.write_all(&[5, 0]).expect("method");
            read_connect_request(&mut socket);
            socket
                .write_all(&[5, 5, 0, 1, 0, 0, 0, 0, 0, 0])
                .expect("refusal");
        });

        let config = config(addr, "", "");
        let result = runtime().block_on(open_stream(
            "target.example",
            2323,
            Some(&config),
            Duration::from_secs(5),
        ));
        let err = match result {
            Ok(_) => panic!("a refused tunnel cannot open"),
            Err(err) => err.to_string(),
        };

        server.join().expect("server thread");
        assert!(
            err.contains("SOCKS5 proxy") && err.contains("target.example:2323"),
            "the error should name the proxy and the target, got: {err}"
        );
    }
}
