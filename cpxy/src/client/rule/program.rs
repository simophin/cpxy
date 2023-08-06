use super::{Action, Program, PropertyAccessor, Rule, Table};
use anyhow::Context;

impl Program {
    fn table_by_name(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|t| t.name == name)
    }

    pub fn run(&self, accessor: &impl PropertyAccessor) -> anyhow::Result<Action> {
        let mut call_chains = vec![self
            .table_by_name("main")
            .context("Unable to find main table")?];

        todo!()
    }
}

impl Table {
    fn run(&self, accessor: &impl PropertyAccessor) -> anyhow::Result<Action> {
        for rule in &self.rules {}

        Ok(Action::Return)
    }
}

impl Rule {
    fn matches(&self, accessor: &impl PropertyAccessor) -> anyhow::Result<bool> {
        for condition in &self.conditions {
            if !condition.run(accessor)? {
                return Ok(Action::Return);
            }
        }

        Ok(self.action.clone())
    }
}
