use std::{
    borrow::Cow,
    fs::File,
    io::{BufRead, BufReader},
    ops::Deref,
    path::Path,
    str::FromStr,
};

use zstd::Encoder;

mod domain_list;
mod geoip;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct CountryCode(pub [u8; 2]);

impl Deref for CountryCode {
    type Target = [u8; 2];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromStr for CountryCode {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != 2 {
            return Err("Invalid country code length");
        }

        let mut buf = [0u8; 2];
        buf.copy_from_slice(s.as_bytes().to_ascii_uppercase().as_ref());

        Ok(Self(buf))
    }
}

impl AsRef<str> for CountryCode {
    fn as_ref(&self) -> &str {
        std::str::from_utf8(&self.0).expect("to be valid utf8")
    }
}

fn main() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dest_folder = project_dir.parent().unwrap().join("cpxy/src");

    let cn: CountryCode = "CN".parse().expect("to parse");

    geoip::download_and_save_geoips(
        &mut Encoder::new(
            File::create(dest_folder.join("geoip/ipv4.zstd")).expect("to create geoip file"),
            0,
        )
        .expect("to create encoder")
        .auto_finish(),
        &mut Encoder::new(
            File::create(dest_folder.join("geoip/ipv6.zstd")).expect("to create geoip file"),
            0,
        )
        .expect("to create encoder")
        .auto_finish(),
        "https://github.com/pmkol/easymosdns/raw/main/rules/china_ip_list.txt",
        |line| Some((line.parse().ok()?, cn)),
    );

    domain_list::download_and_save_domain_list(
        &mut Encoder::new(
            File::create(dest_folder.join("domain_list/bundled_list.zstd"))
                .expect("to create domain list file"),
            0,
        )
        .expect("to create encoder")
        .auto_finish(),
        "https://github.com/pmkol/easymosdns/raw/main/rules/china_domain_list.txt",
        |line| Some((Cow::Borrowed(line), cn)),
    )
}

fn download_and_insert(url: &str, mut for_line: impl FnMut(String) -> ()) {
    BufReader::new(reqwest::blocking::get(url).unwrap())
        .lines()
        .for_each(|r| for_line(r.expect("to read line")));
}
