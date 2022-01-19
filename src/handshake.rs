use crate::parse::ParseError;
use crate::socks5::{
    Address, ClientConnRequest, ClientGreeting, Command, ConnStatusCode, AUTH_NOT_ACCEPTED,
    AUTH_NO_PASSWORD,
};
use crate::utils::{HttpRequest, RWBuffer};
use anyhow::anyhow;
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use url::Url;

struct SocksState {
    auths: Vec<u8>,
}

enum ParseState {
    Init,
    SocksPartial,
    HttpPartial,
}

enum ProxyState {
    SocksGreeted(SocksState),
    Http(HttpRequest),
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "type")]
pub enum ProxyRequest {
    SocksTCP(Address),
    SocksUDP(Address),
    Http(HttpRequest),
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

fn parse_http_request(buf: &[u8]) -> anyhow::Result<Option<(usize, HttpRequest)>> {
    let mut hdr = [httparse::EMPTY_HEADER; 40];
    let mut req = httparse::Request::new(&mut hdr);
    let offset = match req.parse(buf)? {
        httparse::Status::Complete(v) => v,
        _ => return Ok(None),
    };

    Ok(Some((offset, req.try_into()?)))
}

pub async fn handshake_socks5_or_http(
    stream: &mut (impl AsyncRead + AsyncWrite + Unpin),
    buf: &mut RWBuffer,
) -> anyhow::Result<ProxyRequest> {
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
                (Err(_), Err(_)) => {
                    return Err(anyhow!("No socks5/http detected"));
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
        ProxyState::SocksGreeted(s) => handshake_socks5(stream, buf, s).await,
        ProxyState::Http(s) => handshake_http(stream, buf, s).await,
    }
}

async fn handshake_http(
    socks: &mut (impl AsyncRead + AsyncWrite + Unpin),
    buf: &mut RWBuffer,
    r: HttpRequest,
) -> anyhow::Result<ProxyRequest> {
    if r.method.eq_ignore_ascii_case("connect") {
        let address = match Address::from_str(r.path.as_str()) {
            Ok(v) => v,
            Err(e) => {
                socks.write_all(b"HTTP/1.1 400 Invalid URL").await?;
                return Err(e);
            }
        };

        Ok(ProxyRequest::SocksTCP())
    } else {
        Ok(ProxyRequest::Http(r))
    }
}

async fn handshake_socks5(
    socket: &mut (impl AsyncRead + AsyncWrite + Unpin),
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
                    return Ok(ProxyRequest::SocksTCP(address));
                }
                Command::BIND_UDP => {
                    buf.advance_read(offset);
                    return Ok(ProxyRequest::SocksUDP(address));
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
