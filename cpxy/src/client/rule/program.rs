use anyhow::{bail, Context};

use crate::client::rule::op::Op;

use super::{Action, Condition, ExecutionContext, Outcome, Program, Rule, Table};

impl Program {
    fn table_by_name(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name)
    }

    pub fn run(&self, ctx: &impl ExecutionContext) -> anyhow::Result<Outcome> {
        self.run_table("main", ctx)
    }

    fn run_table<'a>(
        &'a self,
        table_name: &str,
        ctx: &impl ExecutionContext,
    ) -> anyhow::Result<Outcome<'a>> {
        let table = self
            .table_by_name(table_name)
            .with_context(|| format!("Unable to find table {table_name}"))?;

        for rule in &table.rules {
            if rule.matches(ctx).context("Matching rule")? {
                match &rule.action {
                    Action::Return => return Ok(Outcome::None),
                    Action::Jump(dst) => {
                        if dst == table_name {
                            bail!("Can not jump to the same table");
                        }

                        match self
                            .run_table(dst, ctx)
                            .with_context(|| format!("Jumping to run table {dst}"))?
                        {
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
    fn matches(&self, accessor: &impl ExecutionContext) -> anyhow::Result<bool> {
        self.conditions.iter().fold(Ok(true), |acc, cond| {
            acc.and_then(|acc| cond.matches(accessor))
        })
    }
}

impl Condition {
    pub fn matches(&self, ctx: &impl ExecutionContext) -> anyhow::Result<bool> {
        let prop = ctx
            .get_property(&self.key)
            .with_context(|| format!("Property {} not found", self.key))?;

        Ok(match &self.op {
            Op::Equals(s) => prop == s,
            Op::NotEquals(s) => prop != s,
            Op::In(list) => ctx.check_value_in(prop, list),
            Op::NotIn(list) => !ctx.check_value_in(prop, list),
            Op::RegexMatches(r) => r.is_match(prop),
        })
    }
}
