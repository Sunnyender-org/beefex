use crate::external_agents::defs::pi;
use crate::external_agents::types::RuntimeAgentDef;

pub const AGENT_DEFS: &[RuntimeAgentDef] = &[pi::PI_AGENT_DEF];

pub fn get_agent_def(id: &str) -> Option<&'static RuntimeAgentDef> {
    AGENT_DEFS.iter().find(|def| def.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_exposes_only_pi() {
        assert_eq!(AGENT_DEFS.len(), 1);
        assert!(get_agent_def("pi").is_some());
        assert!(get_agent_def("claude").is_none());
        assert!(get_agent_def("unknown").is_none());
    }
}
