use anyhow::bail;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Return,
    Jump(String),
    Proxy(String),
    ProxyGroup(String),
    Direct,
    Reject,
}

impl<T: AsRef<str> + Into<String>> TryFrom<(T, T)> for Action {
    type Error = anyhow::Error;

    fn try_from((key, value): (T, T)) -> Result<Self, Self::Error> {
        Ok(match (key.as_ref(), value.as_ref()) {
            ("return", "") => Self::Return,
            ("direct", "") => Self::Direct,
            ("reject", "") => Self::Reject,
            ("jump", v) if !v.is_empty() => Self::Jump(value.into()),
            ("proxy", v) if !v.is_empty() => Self::Proxy(value.into()),
            ("proxygroup", v) if !v.is_empty() => Self::ProxyGroup(value.into()),
            _ => bail!("Invalid action starting with {}", key.as_ref()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_parsing(actual_key: &str, actual_value: &str, expect: Action) {
        let actual: Action = (actual_key, actual_value)
            .try_into()
            .expect("To parse string to action");
        assert_eq!(actual, expect);
    }

    #[test]
    fn serd_works() {
        test_parsing("return", "", Action::Return);
        test_parsing("direct", "", Action::Direct);
        test_parsing("reject", "", Action::Reject);
        test_parsing("jump", "foo", Action::Jump("foo".to_string()));
        test_parsing("proxy", "foo", Action::Proxy("foo".to_string()));
        test_parsing("proxygroup", "foo", Action::ProxyGroup("foo".to_string()));

        Action::try_from(("foo", "bar")).expect_err("To fail parsing");
        Action::try_from(("foo", "")).expect_err("To fail parsing");
    }
}
