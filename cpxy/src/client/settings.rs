use std::collections::HashSet;
use serde::{Deserialize, Serialize};
use crate::protocol::DynamicProtocol;
use crate::rule::RuleString;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ControllerSettings {
    pub fwmark: Option<u32>,
    pub traffic_rules: RuleString,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UpstreamSettings {
    pub name: String,
    pub protocol: DynamicProtocol,
    pub enabled: bool,
    pub groups: Option<HashSet<String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ClientSettings {
    pub controller: ControllerSettings,
    pub upstreams: Vec<UpstreamSettings>,
}