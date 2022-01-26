use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::str::FromStr;
use std::thread::spawn;

trait NumTrait: num_traits::Unsigned + FromStr + Copy {
    fn write_to(self, w: &mut impl Write) -> std::io::Result<()>;
}

impl NumTrait for u32 {
    fn write_to(self, w: &mut impl Write) -> std::io::Result<()> {
        w.write_all(self.to_be_bytes().as_slice())
    }
}

impl NumTrait for u128 {
    fn write_to(self, w: &mut impl Write) -> std::io::Result<()> {
        w.write_all(self.to_be_bytes().as_slice())
    }
}

fn parse_line<N: NumTrait>(s: &str) -> Option<(N, N, &str)> {
    let mut splits = s.trim().trim_end_matches("\n").split(",");
    let from = splits.next()?;
    let to = splits.next()?;
    let code = splits.next()?;
    Some((from.parse().ok()?, to.parse().ok()?, code))
}

fn generate_ip_dat<N: NumTrait>(url: &str, output_file_name: &str) {
    let res = match ureq::get(url).call() {
        Ok(v) if v.status() == 200 => v.into_reader(),
        Ok(v) => {
            panic!("Invalid response: {} for {url}", v.status());
        }
        Err(e) => {
            panic!("Error getting {url}: {e}");
        }
    };

    let mut reader = BufReader::new(res);
    let mut line = String::new();

    let mut writer = BufWriter::new(
        File::create(Path::new(&std::env::var("OUT_DIR").unwrap()).join(output_file_name))
            .expect("To create data file"),
    );

    loop {
        line.clear();
        match reader.read_line(&mut line).expect("To read line") {
            0 => break,
            _ => {}
        };

        if let Some((ip_start, ip_end, country_code)) = parse_line::<N>(line.as_str()) {
            if country_code.as_bytes().len() != 2 {
                eprintln!("Error writing country code: {country_code}");
                continue;
            }

            ip_start.write_to(&mut writer).unwrap();
            ip_end.write_to(&mut writer).unwrap();

            writer.write_all(country_code.as_bytes()).unwrap();
        } else {
            eprintln!("Error paring line: {line}");
        }
    }
}

fn main() {
    let task = spawn(|| {
        generate_ip_dat::<u32>("https://raw.githubusercontent.com/sapics/ip-location-db/master/geo-whois-asn-country/geo-whois-asn-country-ipv4-num.csv", "geoip4.dat")
    });
    generate_ip_dat::<u128>("https://raw.githubusercontent.com/sapics/ip-location-db/master/geo-whois-asn-country/geo-whois-asn-country-ipv6-num.csv", "geoip6.dat");
    task.join().unwrap();
    println!("cargo:rerun-if-changed=build.rs");
}
