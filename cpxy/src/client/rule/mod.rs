mod action;
mod domain;
mod ip;
mod line;
mod op;
mod parser;
mod program;
mod value;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Condition {
    pub key: String,
    pub op: op::Op,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Action {
    Return,
    Jump(String),
    Proxy(String),
    ProxyGroup(String),
    Direct,
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome<'a> {
    None,
    Proxy(&'a str),
    ProxyGroup(&'a str),
    Direct,
    Reject,
}

#[derive(Debug, PartialEq, Eq)]
struct Table {
    pub name: String,
    pub rules: Vec<Rule>,
}

#[derive(Debug, PartialEq, Eq)]
struct Rule {
    pub line_number: usize,
    pub conditions: Vec<Condition>,
    pub action: Action,
}

pub struct Program {
    tables: Vec<Table>,
}

pub trait ExecutionContext {
    fn get_property(&self, key: &str) -> Option<&str>;
    fn check_value_in(&self, value: &str, list_name: &str) -> bool;
}
