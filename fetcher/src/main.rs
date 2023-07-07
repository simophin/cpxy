use std::io::{BufRead, BufReader};

use rusqlite::Connection;

fn main() {
Connection::open_with_flags(path, flags)

    download_and_insert(
        "https://github.com/pmkol/easymosdns/raw/main/rules/china_ip_list.txt",
        |line| {
            println!("Got line {line}");
        },
    )
}

fn download_and_insert(url: &str, for_line: impl Fn(String) -> ()) {
    BufReader::new(reqwest::blocking::get(url).unwrap())
        .lines()
        .for_each(|r| for_line(r.expect("to read line")));
}
