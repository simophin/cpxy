use std::{borrow::Cow, io::Write};

pub fn download_and_save_domain_list(
    writer: &mut impl Write,
    url: &str,
    extract_line: impl Fn(&str) -> Option<(Cow<str>, super::CountryCode)>,
) {
    let mut n = 0usize;

    let mut domains: Vec<(String, super::CountryCode)> = Default::default();

    super::download_and_insert(url, |line| {
        match extract_line(&line) {
            Some((domain, country_code)) => {
                domains.push((domain.to_string(), country_code));
            }
            None => {
                println!("Unknown {line}");
                return;
            }
        };

        n += 1;
    });

    domains.sort_by(|a, b| a.0.cmp(&b.0));

    for (domain, country_code) in domains {
        writeln!(writer, "{domain},{}", country_code.as_ref()).expect("to write domain");
    }

    println!("Processed {n} domains");
}
