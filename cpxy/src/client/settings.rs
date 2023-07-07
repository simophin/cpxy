use crate::protocol::DynamicProtocol;
use crate::rule::RuleString;
use serde::{Deserialize, Serialize};
use std::{collections::HashSet, sync::Arc};

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ControllerSettings {
    pub fwmark: Option<u32>,
    pub traffic_rules: RuleString,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct UpstreamSettings {
    pub id: String,
    pub name: String,
    pub protocol: Arc<DynamicProtocol>,
    pub enabled: bool,
    pub groups: Option<HashSet<String>>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ClientSettings {
    pub controller: ControllerSettings,
    pub upstreams: Vec<UpstreamSettings>,
}
