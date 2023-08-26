use anyhow::{bail, Context};

use crate::client::rule::op::Op;

use super::{Action, Condition, ExecutionContext, Outcome, Program, Rule};

impl Program {
    pub fn run(&self, ctx: &impl ExecutionContext) -> anyhow::Result<Outcome> {
        self.run_table("main", ctx)
            .context("Error running table 'main'")
    }

    fn run_table<'a>(
        &'a self,
        table_name: &str,
        ctx: &impl ExecutionContext,
    ) -> anyhow::Result<Outcome<'a>> {
        let table = self
            .tables
            .iter()
            .find(|t| t.name == table_name)
            .with_context(|| format!("Unable to find table with name == '{table_name}'"))?;

        for rule in &table.rules {
            if rule
                .matches(ctx)
                .with_context(|| format!("Error matching {rule}"))?
            {
                match &rule.action {
                    Action::Return => return Ok(Outcome::None),
                    Action::Jump(dst) => {
                        if dst == table_name {
                            bail!("Can not jump to the same table");
                        }

                        match self.run_table(dst, ctx).with_context(|| {
                            format!("Error running table '{dst}' while executing {rule}")
                        })? {
                            Outcome::None => continue,
                            v => return Ok(v),
                        }
                    }
                    Action::Proxy(p) => return Ok(Outcome::Proxy(p.as_ref())),
                    Action::ProxyGroup(g) => return Ok(Outcome::ProxyGroup(g.as_ref())),
                    Action::Direct => return Ok(Outcome::Direct),
                    Action::Reject => return Ok(Outcome::Reject),
                }
            }
        }

        Ok(Outcome::None)
    }
}

impl Rule {
    fn matches(&self, ctx: &impl ExecutionContext) -> anyhow::Result<bool> {
        for cond in &self.conditions {
            if !cond
                .matches(ctx)
                .with_context(|| format!("Matching condition '{cond}'"))?
            {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

impl Condition {
    pub fn matches(&self, ctx: &impl ExecutionContext) -> anyhow::Result<bool> {
        let prop = match ctx.get_property(&self.key) {
            Some(v) => v,
            None => return Ok(false),
        };

        Ok(match &self.op {
            Op::Equals(s) => prop == s,
            Op::NotEquals(s) => prop != s,
            Op::In(list) => ctx.check_value_in(&self.key, list),
            Op::NotIn(list) => !ctx.check_value_in(&self.key, list),
            Op::RegexMatches(r) => r.is_match(prop),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::regex::Regex;

    use super::*;

    struct MockContext<'a> {
        props: HashMap<&'a str, &'a str>,
        lists: HashMap<&'a str, Vec<&'a str>>,
    }

    impl<'a> ExecutionContext for MockContext<'a> {
        fn available_properties(&self) -> &[&Regex] {
            todo!()
        }

        fn available_list_operations(&self) -> &[(&Regex, &Regex)] {
            todo!()
        }

        fn get_property(&self, key: &str) -> Option<&str> {
            self.props.get(key).map(|s| *s)
        }

        fn check_value_in(&self, key: &str, list_name: &str) -> bool {
            let value = match self.props.get(key) {
                Some(v) => *v,
                None => return false,
            };

            self.lists
                .get(list_name)
                .map(|list| list.contains(&value))
                .unwrap_or(false)
        }
    }

    const PROGRAM: &str = r#"
        main {
            domain in "adblock_list", reject;
            timezone ~= "Asia/.*", jump = "cn";
            jump = "home";
        }

        cn {
            domain in "cn_domains", direct;
            proxy_group = "gfw";  
        }

        home {
            domain in "cn_domains", proxy = "cn-baise";
            geoip == "cn", proxy_group = "cn";
            direct;
        }

    "#;

    fn run_program<'a>(program: &'a Program, props: HashMap<&str, &str>) -> Outcome<'a> {
        let ctx = MockContext {
            props,
            lists: maplit::hashmap! {
                "cn_domains" => vec![
                    "baidu.com",
                    "qq.com",
                ],

                "adblock_list" => vec![
                    "facebook.com",
                ],
            },
        };

        program.run(&ctx).expect("To run program")
    }

    #[test]
    fn adblock_works() {
        let program = PROGRAM.parse().expect("To parse program");
        let outcome = run_program(
            &program,
            maplit::hashmap! {
                "domain" => "facebook.com",
            },
        );

        assert_eq!(outcome, Outcome::Reject);
    }

    #[test]
    fn cn_table_works() {
        let program = PROGRAM.parse().expect("To parse program");

        // CN domain
        let outcome = run_program(
            &program,
            maplit::hashmap! {
                "domain" => "baidu.com",
                "timezone" => "Asia/Shanghai",
            },
        );

        assert_eq!(outcome, Outcome::Direct);

        // Other domains
        let outcome = run_program(
            &program,
            maplit::hashmap! {
                "domain" => "google.com",
                "timezone" => "Asia/Shanghai",
            },
        );
        assert_eq!(outcome, Outcome::ProxyGroup("gfw"));
    }

    #[test]
    fn default_table_works() {
        let program = PROGRAM.parse().expect("To parse program");

        // CN domain
        let outcome = run_program(
            &program,
            maplit::hashmap! {
                "domain" => "baidu.com",
                "timezone" => "Australia/Melbourne",
            },
        );

        assert_eq!(outcome, Outcome::Proxy("cn-baise"));

        // GeoIP CN
        let outcome = run_program(
            &program,
            maplit::hashmap! {
                "geoip" => "cn",
                "timezone" => "Australia/Melbourne",
            },
        );
        assert_eq!(outcome, Outcome::ProxyGroup("cn"));

        // Other
        let outcome = run_program(
            &program,
            maplit::hashmap! {
                "domain" => "google.com",
                "timezone" => "Australia/Melbourne",
            },
        );
        assert_eq!(outcome, Outcome::Direct);
    }
}
