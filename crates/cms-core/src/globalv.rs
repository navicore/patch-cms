use std::collections::BTreeMap;

/// Session-scoped variable storage, modelled after the CMS GLOBALV command.
///
/// Variables are organised into named groups (default: "LASTING").
/// Group and variable names are uppercased; values are stored as-is.
pub struct GlobalVars {
    groups: BTreeMap<String, BTreeMap<String, String>>,
    current_group: String,
}

impl GlobalVars {
    /// Create a new GlobalVars with the default "LASTING" group selected.
    pub fn new() -> Self {
        let mut groups = BTreeMap::new();
        groups.insert("LASTING".to_string(), BTreeMap::new());
        GlobalVars {
            groups,
            current_group: "LASTING".to_string(),
        }
    }

    /// Switch the active group. Creates the group if it doesn't exist.
    pub fn select(&mut self, group: &str) {
        let group = group.to_ascii_uppercase();
        self.groups.entry(group.clone()).or_default();
        self.current_group = group;
    }

    /// Return the name of the currently active group.
    pub fn current_group(&self) -> &str {
        &self.current_group
    }

    /// Set a variable in the current group. Name is uppercased; value is stored as-is.
    pub fn set(&mut self, name: &str, value: &str) {
        let name = name.to_ascii_uppercase();
        let group = self.groups.get_mut(&self.current_group).unwrap();
        group.insert(name, value.to_string());
    }

    /// Get a variable from the current group.
    pub fn get(&self, name: &str) -> Option<&str> {
        let name = name.to_ascii_uppercase();
        self.groups
            .get(&self.current_group)?
            .get(&name)
            .map(|s| s.as_str())
    }

    /// List all variables in the current group, sorted by name (BTreeMap order).
    pub fn list(&self) -> Vec<(&str, &str)> {
        self.list_group(&self.current_group)
    }

    /// List all variables in a specific group, sorted by name.
    pub fn list_group(&self, group: &str) -> Vec<(&str, &str)> {
        let group = group.to_ascii_uppercase();
        match self.groups.get(&group) {
            Some(vars) => vars.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect(),
            None => Vec::new(),
        }
    }

    /// Delete a variable from the current group. Returns true if it existed.
    pub fn delete(&mut self, name: &str) -> bool {
        let name = name.to_ascii_uppercase();
        self.groups
            .get_mut(&self.current_group)
            .map(|g| g.remove(&name).is_some())
            .unwrap_or(false)
    }

    /// Clear all variables in the current group.
    pub fn purge(&mut self) {
        if let Some(group) = self.groups.get_mut(&self.current_group) {
            group.clear();
        }
    }

    /// List all group names, sorted.
    pub fn groups(&self) -> Vec<&str> {
        self.groups.keys().map(|s| s.as_str()).collect()
    }
}

impl Default for GlobalVars {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_group_is_lasting() {
        let gv = GlobalVars::new();
        assert_eq!(gv.current_group(), "LASTING");
    }

    #[test]
    fn set_get_roundtrip() {
        let mut gv = GlobalVars::new();
        gv.set("color", "blue");
        assert_eq!(gv.get("COLOR"), Some("blue"));
    }

    #[test]
    fn name_uppercased_on_set() {
        let mut gv = GlobalVars::new();
        gv.set("myvar", "value");
        assert_eq!(gv.get("MYVAR"), Some("value"));
        assert_eq!(gv.get("myvar"), Some("value"));
    }

    #[test]
    fn value_preserved_as_is() {
        let mut gv = GlobalVars::new();
        gv.set("KEY", "MiXeD CaSe");
        assert_eq!(gv.get("KEY"), Some("MiXeD CaSe"));
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let gv = GlobalVars::new();
        assert_eq!(gv.get("NOVAR"), None);
    }

    #[test]
    fn cross_group_isolation() {
        let mut gv = GlobalVars::new();
        gv.set("X", "lasting-val");
        gv.select("OTHER");
        assert_eq!(gv.get("X"), None);
        gv.set("X", "other-val");
        assert_eq!(gv.get("X"), Some("other-val"));
        gv.select("LASTING");
        assert_eq!(gv.get("X"), Some("lasting-val"));
    }

    #[test]
    fn select_uppercases_group_name() {
        let mut gv = GlobalVars::new();
        gv.select("mygroup");
        assert_eq!(gv.current_group(), "MYGROUP");
    }

    #[test]
    fn select_creates_group_if_absent() {
        let mut gv = GlobalVars::new();
        gv.select("NEWGROUP");
        assert!(gv.groups().contains(&"NEWGROUP"));
    }

    #[test]
    fn delete_existing_var() {
        let mut gv = GlobalVars::new();
        gv.set("DEL", "me");
        assert!(gv.delete("DEL"));
        assert_eq!(gv.get("DEL"), None);
    }

    #[test]
    fn delete_nonexistent_var() {
        let mut gv = GlobalVars::new();
        assert!(!gv.delete("NOVAR"));
    }

    #[test]
    fn purge_clears_current_group() {
        let mut gv = GlobalVars::new();
        gv.set("A", "1");
        gv.set("B", "2");
        gv.purge();
        assert!(gv.list().is_empty());
    }

    #[test]
    fn purge_does_not_affect_other_groups() {
        let mut gv = GlobalVars::new();
        gv.set("X", "lasting");
        gv.select("OTHER");
        gv.set("Y", "other");
        gv.select("LASTING");
        gv.purge();
        assert!(gv.list().is_empty());
        assert_eq!(gv.list_group("OTHER"), vec![("Y", "other")]);
    }

    #[test]
    fn list_sorted_order() {
        let mut gv = GlobalVars::new();
        gv.set("ZEBRA", "z");
        gv.set("APPLE", "a");
        gv.set("MANGO", "m");
        let items = gv.list();
        let names: Vec<&str> = items.iter().map(|(k, _)| *k).collect();
        assert_eq!(names, vec!["APPLE", "MANGO", "ZEBRA"]);
    }

    #[test]
    fn list_group_nonexistent() {
        let gv = GlobalVars::new();
        assert!(gv.list_group("NOSUCH").is_empty());
    }

    #[test]
    fn groups_includes_all() {
        let mut gv = GlobalVars::new();
        gv.select("ALPHA");
        gv.select("BETA");
        let groups = gv.groups();
        assert!(groups.contains(&"LASTING"));
        assert!(groups.contains(&"ALPHA"));
        assert!(groups.contains(&"BETA"));
    }

    #[test]
    fn overwrite_existing_var() {
        let mut gv = GlobalVars::new();
        gv.set("KEY", "old");
        gv.set("KEY", "new");
        assert_eq!(gv.get("KEY"), Some("new"));
    }
}
