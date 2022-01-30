use std::fmt::Debug;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::str::FromStr;
use std::thread::spawn;

trait Num: FromStr + num_traits::Unsigned + Copy + Debug + PartialOrd {
    fn write_be(self, t: &mut impl Write);
}

impl Num for u32 {
    fn write_be(self, t: &mut impl Write) {
        t.write_all(self.to_be_bytes().as_slice()).unwrap();
    }
}

impl Num for u128 {
    fn write_be(self, t: &mut impl Write) {
        t.write_all(self.to_be_bytes().as_slice()).unwrap();
    }
}

fn download_geo_ip<N: Num>(url: &str, file_name: &Path)
where
    <N as FromStr>::Err: Debug,
{
    let mut output = BufWriter::new(File::create(file_name).unwrap());
    let mut res = BufReader::new(ureq::get(url).call().unwrap().into_reader());

    let mut line = Default::default();
    let mut last_start = None;

    while res.read_line(&mut line).unwrap() > 0 {
        {
            let line = line.trim_matches('\n');
            let mut splits = line.split(',');
            let start: N = splits.next().unwrap().parse().unwrap();
            let end: N = splits.next().unwrap().parse().unwrap();
            let code = splits.next().unwrap();
            if code.as_bytes().len() != 2 {
                panic!("Invalid country code {code}");
            }

            match last_start {
                Some(last) if last > start => panic!("Order is wrong!"),
                _ => {}
            };
            last_start = Some(start);

            start.write_be(&mut output);
            end.write_be(&mut output);
            output.write_all(code.as_bytes()).unwrap();
        }

        line.clear();
    }
}

fn main() {
    env_logger::init();
    let mut args = std::env::args();
    let _ = args.next();
    let out_dir = args.next().expect("OUT_DIR to be the first argument");

    let download_ipv4 = {
        let out_dir = out_dir.clone();
        spawn(move || {
            download_geo_ip::<u32>(
                "https://raw.githubusercontent.com/sapics/ip-location-db/master/geo-whois-asn-country/geo-whois-asn-country-ipv4-num.csv",
                &Path::new(out_dir.as_str()).join("ipv4.dat"))
        })
    };

    let download_ipv6 = {
        let out_dir = out_dir.clone();
        spawn(move || {
            download_geo_ip::<u128>(
                "https://raw.githubusercontent.com/sapics/ip-location-db/master/geo-whois-asn-country/geo-whois-asn-country-ipv6-num.csv",
                &Path::new(out_dir.as_str()).join("ipv6.dat"))
        })
    };

    download_ipv4.join().unwrap();
    download_ipv6.join().unwrap();
}
