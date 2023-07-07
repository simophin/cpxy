use std::{
    io::{BufRead, BufReader},
    path::Path,
};

use rusqlite::{Connection, OpenFlags};

fn main() {
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let geo_path = project_dir.parent().unwrap().join("cpxy/src/geo.sqlitedb");

    let conn = Connection::open_with_flags(&geo_path, OpenFlags::SQLITE_OPEN_CREATE)
        .expect("To create db");

    // Drop if any and re-create table for ip address ranges with start and end
    conn.execute_batch(
        "DROP TABLE IF EXISTS geoips;
         CREATE TABLE geoips (
             start INTEGER NOT NULL,
             end INTEGER NOT NULL,
             v4 BOOL NOT NULL,
             country TEXT NOT NULL
         );",
    )
    .expect("Creating geoips");

    download_and_insert(
        "https://github.com/pmkol/easymosdns/raw/main/rules/china_ip_list.txt",
        |line| {
            // Parse the line as IP network and insert into database as ip ranges with country = CN
            
        },
    )
}

fn download_and_insert(url: &str, for_line: impl Fn(String) -> ()) {
    BufReader::new(reqwest::blocking::get(url).unwrap())
        .lines()
        .for_each(|r| for_line(r.expect("to read line")));
}
