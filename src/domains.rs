use crate::socks5::Address;
use adblock::blocker::BlockerResult;
use adblock::engine::Engine;
use lazy_static::lazy_static;

fn new_engine() -> Engine {
    let mut engine = Engine::new(true);
    engine
        .deserialize(include_bytes!(concat!(env!("OUT_DIR"), "/gfwlist.abp")))
        .expect("To deserialize engine");
    engine
}

pub fn matches_gfw(address: &Address) -> bool {
    lazy_static! {
        static ref ENGINE: Engine = new_engine();
    }
    let url = match address.get_port() {
        80 => format!("http://{address}"),
        443 => format!("https://{address}"),
        _ => address.to_string(),
    };
    match ENGINE.check_network_urls(url.as_str(), url.as_str(), "") {
        BlockerResult { matched, .. } if matched => true,
        _ => false,
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_engine() {
        assert!(matches_gfw(&"www.google.com:443".parse().unwrap()));
        assert!(matches_gfw(&"www.google.com:443".parse().unwrap()));
        assert!(matches_gfw(&"twitter.com:80".parse().unwrap()));
        assert!(!matches_gfw(&"qq.com:443".parse().unwrap()));
    }
}
