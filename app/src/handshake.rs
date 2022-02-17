use crate::http::HttpRequest;
use crate::parse::ParseError;
use crate::proxy::protocol::ProxyRequest;
use crate::socks4::{
    self, parse_socks4_request, respond_socks4, SOCKS4_REPLY_FAILED, SOCKS4_REPLY_GRANTED,
    SOCKS4_REQUEST_TCP,
};
use crate::socks5::{
    Address, ClientConnRequest, ClientGreeting, Command, ConnStatusCode, AUTH_NOT_ACCEPTED,
    AUTH_NO_PASSWORD,
};
use crate::url::HttpUrl;
use crate::utils::RWBuffer;
use anyhow::{anyhow, bail, Context};
use futures_lite::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use std::borrow::Cow;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};

struct SocksState {
    auths: Vec<u8>,
}

enum ParseState {
    Init,
    Socks5Partial,
    Socks4Partial,
    HttpPartial,
}

enum ProxyState {
    Socks5Greeted(SocksState),
    Socks4(socks4::Request<'static>),
    Http(HttpRequest<'static>),
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

enum HandshakeType {
    Socks5,
    Socks4,
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
                    parse_socks4_request(buf.read_buf()),
                    HttpRequest::parse(buf.read_buf()),
                ) {
                    (Ok(Some((offset, s))), _, _) => {
                        proxy_state = ProxyState::Socks5Greeted(s);
                        buf.advance_read(offset);
                        break;
                    }
                    (_, Ok(Some((offset, r))), _) => {
                        proxy_state = ProxyState::Socks4(socks4::Request {
                            cmd: r.cmd,
                            addr: r.addr.into_owned(),
                        });
                        buf.advance_read(offset);
                        break;
                    }
                    (_, _, Ok(Some((offset, h)))) => {
                        proxy_state = ProxyState::Http(h);
                        buf.advance_read(offset);
                        break;
                    }
                    (Err(e1), Err(e2), Err(e3)) => {
                        return Err(anyhow!(
                            "No socks5/http detected: SOCK5: {e1:?}, SOCK4: {e2:?} HTTP: {e3:?}"
                        ));
                    }
                    (Err(_), Err(_), Ok(None)) => parse_state = ParseState::HttpPartial,
                    (Ok(None), Err(_), Err(_)) => parse_state = ParseState::Socks5Partial,
                    (Err(_), Ok(None), Err(_)) => parse_state = ParseState::Socks4Partial,
                    _ => {}
                },

                ParseState::HttpPartial => match HttpRequest::parse(buf.read_buf())? {
                    None => {}
                    Some((offset, h)) => {
                        proxy_state = ProxyState::Http(h);
                        buf.advance_read(offset);
                        break;
                    }
                },

                ParseState::Socks5Partial => match parse_socks5_greeting(buf.read_buf())? {
                    None => {}
                    Some((offset, h)) => {
                        proxy_state = ProxyState::Socks5Greeted(h);
                        buf.advance_read(offset);
                        break;
                    }
                },
                ParseState::Socks4Partial => match parse_socks4_request(buf.read_buf())? {
                    None => {}
                    Some((offset, r)) => {
                        proxy_state = ProxyState::Socks4(socks4::Request {
                            cmd: r.cmd,
                            addr: r.addr.into_owned(),
                        });
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
            ProxyState::Socks5Greeted(s) => Ok((
                Handshaker(HandshakeType::Socks5),
                handshake_socks5(stream, buf, s).await?,
            )),
            ProxyState::Http(s) => {
                let req = handshake_http(s)?;
                let handshaker = Self(match &req {
                    ProxyRequest::TCP { .. } => HandshakeType::HttpTcpChannel,
                    ProxyRequest::HTTP { .. } => HandshakeType::Http,
                    _ => unreachable!("Unknown proxy request type for http proxy"),
                });
                Ok((handshaker, req))
            }
            ProxyState::Socks4(req) => Ok((Self(HandshakeType::Socks4), handshake_socks4(req)?)),
        }
    }

    pub async fn respond_ok(
        self,
        stream: &mut (impl AsyncWrite + Send + Sync + Unpin),
        bound_address: Option<SocketAddr>,
    ) -> anyhow::Result<()> {
        match (self.0, bound_address) {
            (HandshakeType::Socks5, Some(bound_address)) => {
                ClientConnRequest::respond(
                    stream,
                    ConnStatusCode::GRANTED,
                    &Address::IP(bound_address),
                )
                .await
            }
            (HandshakeType::Socks5, None) => {
                ClientConnRequest::respond(stream, ConnStatusCode::FAILED, &Address::default())
                    .await?;
                bail!("Bound address is required for socks5")
            }
            (HandshakeType::Socks4, Some(SocketAddr::V4(addr))) => {
                respond_socks4(stream, &addr, SOCKS4_REPLY_GRANTED).await
            }
            (HandshakeType::Socks4, _) => {
                respond_socks4(
                    stream,
                    &SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
                    SOCKS4_REPLY_GRANTED,
                )
                .await
            }
            (HandshakeType::Http, _) => Ok(()),
            (HandshakeType::HttpTcpChannel, _) => {
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
            HandshakeType::Socks4 => {
                respond_socks4(
                    stream,
                    &SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
                    SOCKS4_REPLY_FAILED,
                )
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

fn handshake_socks4(
    socks4::Request { addr, cmd }: socks4::Request<'static>,
) -> anyhow::Result<ProxyRequest> {
    if cmd != SOCKS4_REQUEST_TCP {
        bail!("Unsupported socks4 request: {cmd}");
    }

    Ok(ProxyRequest::TCP { dst: addr })
}

fn handshake_http(r: HttpRequest<'static>) -> anyhow::Result<ProxyRequest> {
    match r {
        HttpRequest { path, method, .. } if method.eq_ignore_ascii_case("connect") => {
            Ok(ProxyRequest::TCP { dst: path.parse()? })
        }
        HttpRequest {
            path,
            method,
            common,
        } => {
            let HttpUrl {
                is_https,
                address,
                path,
            } = HttpUrl::try_from(path.as_ref()).context("Parsing HTTP request path")?;
            Ok(ProxyRequest::HTTP {
                dst: address.into_owned(),
                https: is_https,
                req: HttpRequest {
                    method: Cow::Owned(method.into_owned()),
                    path: Cow::Owned(path.into_owned()),
                    common,
                },
            })
        }
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
                    let dst = address.into_owned();
                    buf.advance_read(offset);
                    return Ok(ProxyRequest::TCP { dst });
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
