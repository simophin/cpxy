use super::{Action, ExecutionContext, Outcome, Program, Rule, Table};
use anyhow::Context;

impl Program {
    fn table_by_name(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name)
    }

    pub fn run(&self, ctx: &impl ExecutionContext) -> anyhow::Result<Outcome> {
        let mut call_chains = vec![self
            .table_by_name("main")
            .context("Unable to find main table")?];

        todo!()
    }
}

impl Table {
    fn run(&self, ctx: &impl ExecutionContext) -> anyhow::Result<Action> {
        for rule in &self.rules {}

        Ok(Action::Return)
    }
}

impl Rule {
    fn matches(&self, accessor: &impl ExecutionContext) -> anyhow::Result<bool> {
        todo!()
    }
}
