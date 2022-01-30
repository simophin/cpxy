use crate::socks5::Address;
use adblock::engine::Engine;
use anyhow::anyhow;
use lazy_static::lazy_static;

fn create_engine(data: &[u8]) -> anyhow::Result<Engine> {
    let mut engine = Engine::new(true);
    engine
        .deserialize(data)
        .map_err(|e| anyhow!("Error init adblock engine: {e:?}"))?;
    Ok(engine)
}

pub fn matches_gfw_list(addr: &Address) -> bool {
    lazy_static! {
        static ref ENGINE: Engine = create_engine(include_bytes!("gfw_list.dat")).unwrap();
    }

    let url = match addr.get_port() {
        443 => format!("https://{}", addr.get_host()),
        _ => format!("http://{}", addr.get_host()),
    };

    ENGINE
        .check_network_urls(url.as_str(), url.as_str(), "")
        .matched
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_match_gfw_list() {
        assert!(matches_gfw_list(&"www.google.com:443".parse().unwrap()));
        assert!(matches_gfw_list(&"www.facebook.com:80".parse().unwrap()));
        assert!(matches_gfw_list(&"twitter.com:22".parse().unwrap()));
        assert!(!matches_gfw_list(&"www.qq.com:443".parse().unwrap()));
    }
}
