use crate::parse::ParseError;
use crate::proxy::protocol::ProxyRequest;
use crate::socks5::{
    Address, ClientConnRequest, ClientGreeting, Command, ConnStatusCode, AUTH_NOT_ACCEPTED,
    AUTH_NO_PASSWORD,
};
use crate::utils::RWBuffer;
use anyhow::anyhow;
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use std::io::Write;
use std::net::SocketAddr;

struct SocksState {
    auths: Vec<u8>,
}

enum ParseState {
    Init,
    SocksPartial,
    HttpPartial,
}

struct HttpProxyState {
    address: Address,
    method: String,
    headers: Vec<u8>,
}

impl HttpProxyState {
    pub fn from_http(r: &httparse::Request<'_, '_>) -> anyhow::Result<Self> {
        let path = r.path.ok_or_else(|| anyhow!("No path found"))?;
        let method = r.method.ok_or_else(|| anyhow!("No method found"))?;
        let protocol = method.find("://").map(|index| &path[..index]).unwrap_or("");

        let ((host_and_port, path), default_port) = if protocol.eq_ignore_ascii_case("http") {
            let path = &path["http://".len()..];
            match path.find("/") {
                Some(index) => (path.split_at(index), Some(80u16)),
                None => ((path, ""), Some(80)),
            }
        } else if protocol.eq_ignore_ascii_case("https") {
            let path = &path["https://".len()..];
            match path.find("/") {
                Some(index) => (path.split_at(index), Some(443)),
                None => ((path, ""), Some(443)),
            }
        } else if protocol.is_empty() {
            ((path, ""), None)
        } else {
            return Err(anyhow!("Unsupported protocol: {protocol}",));
        };

        let (host, port) = match host_and_port.rfind(":") {
            Some(index) => host_and_port.split_at(index),
            None => (host_and_port, ":"),
        };

        let port = match (&port[1..]).parse::<u16>() {
            Ok(v) => v,
            Err(_) if port == ":" && default_port.is_some() => default_port.unwrap(),
            Err(e) => return Err(anyhow!("Invalid port {port}: {e}")),
        };

        let address = format!("{host}:{port}");

        let mut headers = Vec::new();
        write!(&mut headers, "{method} {path} HTTP/1.1\r\n")?;

        let mut has_host = false;
        for hdr in r.headers.iter() {
            if hdr.name.eq_ignore_ascii_case("host") {
                has_host = true
            }
            headers.extend_from_slice(hdr.name.as_bytes());
            headers.extend_from_slice(b": ");
            headers.extend_from_slice(hdr.value);
            headers.extend_from_slice(b"\r\n");
        }

        if !has_host {
            headers.extend_from_slice(b"Host: ");
            headers.extend_from_slice(address.as_bytes());
            headers.extend_from_slice(b"\r\n");
        }
        headers.extend_from_slice(b"\r\n");
        Ok(Self {
            address: address.parse()?,
            method: method.to_string(),
            headers,
        })
    }
}

enum ProxyState {
    SocksGreeted(SocksState),
    Http(HttpProxyState),
}

fn parse_socks5_greeting(buf: &[u8]) -> Result<Option<(usize, SocksState)>, ParseError> {
    match ClientGreeting::parse(buf)? {
        None => Ok(None),
        Some((offset, ClientGreeting { auths })) => Ok(Some((
            offset,
            SocksState {
                auths: auths.to_vec(),
            },
        ))),
    }
}

fn parse_http_request(buf: &[u8]) -> anyhow::Result<Option<(usize, HttpProxyState)>> {
    let mut hdr = [httparse::EMPTY_HEADER; 40];
    let mut req = httparse::Request::new(&mut hdr);
    let offset = match req.parse(buf)? {
        httparse::Status::Complete(v) => v,
        _ => return Ok(None),
    };

    let v = HttpProxyState::from_http(&req)?;
    Ok(Some((offset, v)))
}

enum HandshakeType {
    Socks5,
    Http,
    HttpTcpChannel,
}

pub struct Handshaker(HandshakeType);

impl Handshaker {
    pub async fn start(
        stream: &mut (impl AsyncRead + AsyncWrite + Send + Sync + Unpin),
        buf: &mut RWBuffer,
    ) -> anyhow::Result<(Handshaker, ProxyRequest)> {
        let mut parse_state = ParseState::Init;
        let proxy_state: ProxyState;

        loop {
            match &parse_state {
                ParseState::Init => match (
                    parse_socks5_greeting(buf.read_buf()),
                    parse_http_request(buf.read_buf()),
                ) {
                    (Ok(None), Ok(None)) => {}
                    (Ok(Some((offset, s))), _) => {
                        proxy_state = ProxyState::SocksGreeted(s);
                        buf.advance_read(offset);
                        break;
                    }
                    (_, Ok(Some((offset, h)))) => {
                        proxy_state = ProxyState::Http(h);
                        buf.advance_read(offset);
                        break;
                    }
                    (Err(e1), Err(e2)) => {
                        return Err(anyhow!("No socks5/http detected: {e1}, {e2}"));
                    }
                    (Err(_), Ok(None)) => parse_state = ParseState::HttpPartial,
                    (Ok(None), Err(_)) => parse_state = ParseState::SocksPartial,
                },

                ParseState::HttpPartial => match parse_http_request(buf.read_buf())? {
                    None => {}
                    Some((offset, h)) => {
                        proxy_state = ProxyState::Http(h);
                        buf.advance_read(offset);
                        break;
                    }
                },

                ParseState::SocksPartial => match parse_socks5_greeting(buf.read_buf())? {
                    None => {}
                    Some((offset, h)) => {
                        proxy_state = ProxyState::SocksGreeted(h);
                        buf.advance_read(offset);
                        break;
                    }
                },
            };

            match stream.read(buf.write_buf()).await? {
                0 => return Err(anyhow!("Unexpected EOF")),
                v => buf.advance_write(v),
            };
        }

        match proxy_state {
            ProxyState::SocksGreeted(s) => Ok((
                Handshaker(HandshakeType::Socks5),
                handshake_socks5(stream, buf, s).await?,
            )),
            ProxyState::Http(s) => {
                let req = handshake_http(s).await?;
                let handshaker = Self(match &req {
                    ProxyRequest::TCP { .. } => HandshakeType::HttpTcpChannel,
                    ProxyRequest::Http { .. } => HandshakeType::Http,
                    _ => unreachable!("Unknown proxy request type for http proxy"),
                });
                Ok((handshaker, req))
            }
        }
    }

    pub async fn respond_ok(
        self,
        stream: &mut (impl AsyncWrite + Send + Sync + Unpin),
        bound_address: SocketAddr,
    ) -> anyhow::Result<()> {
        match self.0 {
            HandshakeType::Socks5 => {
                ClientConnRequest::respond(
                    stream,
                    ConnStatusCode::GRANTED,
                    &Address::IP(bound_address),
                )
                .await
            }
            HandshakeType::Http => Ok(()),
            HandshakeType::HttpTcpChannel => {
                stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").await?;
                Ok(())
            }
        }
    }

    pub async fn respond_err(
        self,
        stream: &mut (impl AsyncWrite + Send + Sync + Unpin),
    ) -> anyhow::Result<()> {
        match self.0 {
            HandshakeType::Socks5 => {
                ClientConnRequest::respond(stream, ConnStatusCode::FAILED, &Default::default())
                    .await
            }
            _ => {
                stream
                    .write_all(b"HTTP/1.1 500 Internal server error\r\n\r\n")
                    .await?;
                Ok(())
            }
        }
    }
}

async fn handshake_http(
    HttpProxyState {
        address,
        method,
        headers,
    }: HttpProxyState,
) -> anyhow::Result<ProxyRequest> {
    if method.eq_ignore_ascii_case("connect") {
        Ok(ProxyRequest::TCP{dst: address})
    } else {
        Ok(ProxyRequest::Http {
            dst: address,
            request: headers.into(),
        })
    }
}

async fn handshake_socks5(
    socket: &mut (impl AsyncRead + AsyncWrite + Send + Sync + Unpin),
    buf: &mut RWBuffer,
    state: SocksState,
) -> anyhow::Result<ProxyRequest> {
    if !state.auths.contains(&AUTH_NO_PASSWORD) {
        ClientGreeting::respond(AUTH_NOT_ACCEPTED, socket).await?;
        return Err(anyhow!("Invalid socks auth method"));
    }

    ClientGreeting::respond(AUTH_NO_PASSWORD, socket).await?;

    loop {
        match ClientConnRequest::parse(buf.read_buf())? {
            None => {}
            Some((offset, ClientConnRequest { cmd, address })) => match cmd {
                Command::CONNECT_TCP => {
                    buf.advance_read(offset);
                    return Ok(ProxyRequest::TCP {
                        dst: address,
                    });
                }
                Command::BIND_UDP => {
                    buf.advance_read(offset);
                    return Ok(ProxyRequest::UDP);
                }
                _ => {
                    ClientConnRequest::respond(
                        socket,
                        ConnStatusCode::UNSUPPORTED_COMMAND,
                        &Default::default(),
                    )
                    .await?;
                    return Err(anyhow!("Invalid socks5 command"));
                }
            },
        };

        match socket.read(buf.write_buf()).await? {
            0 => return Err(anyhow!("Unexpected EOF")),
            v => buf.advance_write(v),
        };
    }
}
