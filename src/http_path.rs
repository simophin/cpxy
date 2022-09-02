use std::str::FromStr;

use anyhow::Context;
use smallvec::SmallVec;

pub struct HttpPath {
    pub path: String,
    pub queries: SmallVec<[(String, String); 3]>,
}

impl FromStr for HttpPath {
    type Err = anyhow::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (path, queries_str) = match value.split_once('?') {
            Some(v) => v,
            None => {
                return Ok(HttpPath {
                    path: value.to_string(),
                    queries: Default::default(),
                })
            }
        };

        let mut queries = SmallVec::new();

        for query in queries_str.split('&') {
            match query.split_once('=') {
                Some((name, value)) => {
                    queries.push((
                        urlencoding::decode(name)
                            .with_context(|| format!("Parsing query parameter: {name}"))?
                            .into_owned(),
                        urlencoding::decode(value)
                            .with_context(|| format!("Parsing query parameter: {value}"))?
                            .into_owned(),
                    ));
                }
                None => {
                    queries.push((
                        urlencoding::decode(query)
                            .with_context(|| format!("Parsing query parameter: {query}"))?
                            .into_owned(),
                        Default::default(),
                    ));
                }
            };
        }

        Ok(Self {
            path: path.to_string(),
            queries,
        })
    }
}
