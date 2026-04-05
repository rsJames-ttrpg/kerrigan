use std::collections::HashMap;

use crate::tools::PermissionLevel;

#[derive(Debug, Clone)]
pub enum PermissionMode {
    AllowAll,
    DenyUnknown,
}

#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    pub mode: PermissionMode,
    pub overrides: HashMap<String, PermissionLevel>,
}

impl PermissionPolicy {
    pub fn allow_all() -> Self {
        Self {
            mode: PermissionMode::AllowAll,
            overrides: HashMap::new(),
        }
    }

    pub fn deny_unknown() -> Self {
        Self {
            mode: PermissionMode::DenyUnknown,
            overrides: HashMap::new(),
        }
    }

    pub fn with_override(mut self, tool_name: &str, level: PermissionLevel) -> Self {
        self.overrides.insert(tool_name.to_string(), level);
        self
    }

    pub fn is_allowed(&self, tool_name: &str, tool_permission: PermissionLevel) -> bool {
        if let Some(override_level) = self.overrides.get(tool_name) {
            return *override_level >= tool_permission;
        }
        matches!(self.mode, PermissionMode::AllowAll)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allow_all_permits_everything() {
        let policy = PermissionPolicy::allow_all();
        assert!(policy.is_allowed("bash", PermissionLevel::FullAccess));
        assert!(policy.is_allowed("read_file", PermissionLevel::ReadOnly));
    }

    #[test]
    fn test_deny_unknown_blocks_unregistered() {
        let policy = PermissionPolicy::deny_unknown();
        assert!(!policy.is_allowed("bash", PermissionLevel::FullAccess));
        assert!(!policy.is_allowed("read_file", PermissionLevel::ReadOnly));
    }

    #[test]
    fn test_override_allows_specific_tool() {
        let policy =
            PermissionPolicy::deny_unknown().with_override("bash", PermissionLevel::FullAccess);
        assert!(policy.is_allowed("bash", PermissionLevel::FullAccess));
        assert!(!policy.is_allowed("read_file", PermissionLevel::ReadOnly));
    }

    #[test]
    fn test_override_level_must_be_sufficient() {
        let policy =
            PermissionPolicy::deny_unknown().with_override("bash", PermissionLevel::ReadOnly);
        // ReadOnly < FullAccess, so it should be denied
        assert!(!policy.is_allowed("bash", PermissionLevel::FullAccess));
        assert!(policy.is_allowed("bash", PermissionLevel::ReadOnly));
    }
}
