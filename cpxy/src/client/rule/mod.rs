use num_traits::Num;

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
    pub value: String,
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

#[derive(Debug, PartialEq, Eq)]
struct Table {
    pub name: String,
    pub rules: Vec<Rule>,
}

#[derive(Debug, PartialEq, Eq)]
struct Rule {
    pub conditions: Vec<Condition>,
    pub action: Action,
}

pub struct Program {
    tables: Vec<Table>,
}

pub enum PropertyValue<'a> {
    String(&'a str),
    IPNetwork(ipnetwork::IpNetwork),
    List(&'a [&'a PropertyValue<'a>]),
}

pub trait PropertyAccessor {
    fn get(&self, key: &str) -> Option<&PropertyValue<'_>>;
}
