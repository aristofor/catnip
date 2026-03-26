// FILE: catnip_repl/src/config_editor.rs
//! Interactive config editor overlay for the REPL.

/// Logical grouping of config keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigGroup {
    Execution,
    Display,
    Cache,
    Debug,
    Format,
    Repl,
}

impl ConfigGroup {
    pub fn label(&self) -> &'static str {
        match self {
            ConfigGroup::Execution => "execution",
            ConfigGroup::Display => "display",
            ConfigGroup::Cache => "cache",
            ConfigGroup::Debug => "debug",
            ConfigGroup::Format => "format",
            ConfigGroup::Repl => "repl",
        }
    }
}

/// Group definition: order and keys.
pub struct GroupDef {
    pub group: ConfigGroup,
    pub keys: &'static [&'static str],
}

pub const GROUPS: &[GroupDef] = &[
    GroupDef {
        group: ConfigGroup::Execution,
        keys: &["executor", "optimize", "tco", "jit"],
    },
    GroupDef {
        group: ConfigGroup::Display,
        keys: &["no_color", "theme"],
    },
    GroupDef {
        group: ConfigGroup::Cache,
        keys: &["enable_cache", "cache_max_size_mb", "cache_ttl_seconds"],
    },
    GroupDef {
        group: ConfigGroup::Debug,
        keys: &["log_weird_errors", "max_weird_logs", "memory_limit"],
    },
    GroupDef {
        group: ConfigGroup::Format,
        keys: &["indent_size", "line_length"],
    },
    GroupDef {
        group: ConfigGroup::Repl,
        keys: &["show_parse_time", "show_exec_time", "debug_mode", "max_history"],
    },
];

/// Type constraint for a config key.
#[derive(Debug, Clone)]
pub enum ConfigType {
    Bool,
    Int { min: i64, max: i64 },
    Choice(&'static [&'static str]),
}

/// Config key metadata (type constraints, defaults).
struct KeyMeta {
    key: &'static str,
    config_type: ConfigType,
    default_display: &'static str,
    is_format: bool,
    is_repl: bool,
}

const ALL_KEYS: &[KeyMeta] = &[
    KeyMeta {
        key: "executor",
        config_type: ConfigType::Choice(&["vm", "ast"]),
        default_display: "'vm'",
        is_format: false,
        is_repl: false,
    },
    KeyMeta {
        key: "optimize",
        config_type: ConfigType::Int { min: 0, max: 3 },
        default_display: "3",
        is_format: false,
        is_repl: false,
    },
    KeyMeta {
        key: "tco",
        config_type: ConfigType::Bool,
        default_display: "True",
        is_format: false,
        is_repl: false,
    },
    KeyMeta {
        key: "jit",
        config_type: ConfigType::Bool,
        default_display: "False",
        is_format: false,
        is_repl: false,
    },
    KeyMeta {
        key: "no_color",
        config_type: ConfigType::Bool,
        default_display: "False",
        is_format: false,
        is_repl: false,
    },
    KeyMeta {
        key: "theme",
        config_type: ConfigType::Choice(&["auto", "dark", "light"]),
        default_display: "'auto'",
        is_format: false,
        is_repl: false,
    },
    KeyMeta {
        key: "enable_cache",
        config_type: ConfigType::Bool,
        default_display: "True",
        is_format: false,
        is_repl: false,
    },
    KeyMeta {
        key: "cache_max_size_mb",
        config_type: ConfigType::Int { min: 0, max: 10000 },
        default_display: "100",
        is_format: false,
        is_repl: false,
    },
    KeyMeta {
        key: "cache_ttl_seconds",
        config_type: ConfigType::Int { min: 0, max: 999999 },
        default_display: "86400",
        is_format: false,
        is_repl: false,
    },
    KeyMeta {
        key: "log_weird_errors",
        config_type: ConfigType::Bool,
        default_display: "True",
        is_format: false,
        is_repl: false,
    },
    KeyMeta {
        key: "max_weird_logs",
        config_type: ConfigType::Int { min: 0, max: 10000 },
        default_display: "50",
        is_format: false,
        is_repl: false,
    },
    KeyMeta {
        key: "memory_limit",
        config_type: ConfigType::Int { min: 0, max: 65536 },
        default_display: "2048",
        is_format: false,
        is_repl: false,
    },
    KeyMeta {
        key: "indent_size",
        config_type: ConfigType::Int { min: 1, max: 16 },
        default_display: "4",
        is_format: true,
        is_repl: false,
    },
    KeyMeta {
        key: "line_length",
        config_type: ConfigType::Int { min: 40, max: 500 },
        default_display: "120",
        is_format: true,
        is_repl: false,
    },
    KeyMeta {
        key: "show_parse_time",
        config_type: ConfigType::Bool,
        default_display: "false",
        is_format: false,
        is_repl: true,
    },
    KeyMeta {
        key: "show_exec_time",
        config_type: ConfigType::Bool,
        default_display: "false",
        is_format: false,
        is_repl: true,
    },
    KeyMeta {
        key: "debug_mode",
        config_type: ConfigType::Bool,
        default_display: "false",
        is_format: false,
        is_repl: true,
    },
    KeyMeta {
        key: "max_history",
        config_type: ConfigType::Int { min: 100, max: 100000 },
        default_display: "1000",
        is_format: false,
        is_repl: true,
    },
];

/// One item in the config editor list.
#[derive(Debug, Clone)]
pub struct ConfigItem {
    pub key: String,
    pub value: String,
    pub default_value: String,
    pub source: String,
    pub config_type: ConfigType,
    pub group: ConfigGroup,
    pub is_format: bool,
    pub is_repl: bool,
}

impl ConfigItem {
    pub fn is_modified(&self) -> bool {
        self.value != self.default_value
    }
}

/// A row in the rendered view (either a group header or an item).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    GroupHeader(usize), // index into GROUPS
    Item(usize),        // index into self.items
}

/// Inline edit mode for Int values.
#[derive(Debug, Clone)]
pub struct EditMode {
    pub buffer: String,
    pub cursor: usize,
}

/// Action produced by the editor for the REPL to apply.
#[derive(Debug)]
pub enum ConfigAction {
    SetValue {
        key: String,
        value: String,
        is_format: bool,
    },
    SetRepl {
        key: String,
        value: String,
    },
}

/// Status message type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatusKind {
    Success,
    Error,
    Info,
}

/// Interactive config editor state.
pub struct ConfigEditorState {
    pub active: bool,
    pub items: Vec<ConfigItem>,
    pub selected: usize,
    pub edit: Option<EditMode>,
    pub status_message: Option<(String, StatusKind)>,
    pub scroll_offset: usize,
    pub show_help: bool,
    pub title: String,
}

impl ConfigEditorState {
    pub fn new() -> Self {
        Self {
            active: false,
            items: Vec::new(),
            selected: 0,
            edit: None,
            status_message: None,
            scroll_offset: 0,
            show_help: false,
            title: String::new(),
        }
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.items.clear();
        self.selected = 0;
        self.edit = None;
        self.status_message = None;
        self.scroll_offset = 0;
        self.show_help = false;
        self.title.clear();
    }

    /// Populate items from ConfigManager + REPL data.
    /// `entries`: Vec<(key, value_repr, source_str)> for all keys.
    /// `repl_entries`: Vec<(key, value_repr)> for REPL-local keys (source always "session").
    pub fn load(&mut self, entries: Vec<(String, String, String)>, repl_entries: Vec<(String, String)>) {
        self.items.clear();
        self.selected = 0;
        self.edit = None;
        self.status_message = None;
        self.scroll_offset = 0;
        self.show_help = false;

        // Build items ordered by GROUPS
        for gdef in GROUPS {
            for &gkey in gdef.keys {
                let meta = ALL_KEYS.iter().find(|m| m.key == gkey);

                // Check REPL entries first, then config entries
                let (value, source) = if let Some((_, v)) = repl_entries.iter().find(|(k, _)| k == gkey) {
                    (v.clone(), "session".to_string())
                } else if let Some((_, v, s)) = entries.iter().find(|(k, _, _)| k == gkey) {
                    (v.clone(), s.clone())
                } else {
                    let default = meta.map(|m| m.default_display).unwrap_or("?");
                    (default.to_string(), "default".to_string())
                };

                let (config_type, default_display, is_format, is_repl) = match meta {
                    Some(m) => (
                        m.config_type.clone(),
                        m.default_display.to_string(),
                        m.is_format,
                        m.is_repl,
                    ),
                    None => (ConfigType::Bool, value.clone(), false, false),
                };

                self.items.push(ConfigItem {
                    key: gkey.to_string(),
                    value,
                    default_value: default_display,
                    source,
                    config_type,
                    group: gdef.group,
                    is_format,
                    is_repl,
                });
            }
        }

        self.active = true;
    }

    pub fn total_items(&self) -> usize {
        self.items.len()
    }

    /// Build the row model for rendering.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        let mut current_group: Option<ConfigGroup> = None;

        for (i, item) in self.items.iter().enumerate() {
            if current_group != Some(item.group) {
                let gi = GROUPS.iter().position(|g| g.group == item.group).unwrap_or(0);
                rows.push(Row::GroupHeader(gi));
                current_group = Some(item.group);
            }
            rows.push(Row::Item(i));
        }
        rows
    }

    /// Height of the overlay (rows + status + help).
    pub fn visible_lines(&self) -> usize {
        if !self.active || self.items.is_empty() {
            return 0;
        }
        let rows = self.rows();
        let viewport_rows = rows.len().min(30);
        // visible rows + status + help
        viewport_rows + 2
    }

    /// Visible rows capped by viewport height.
    pub fn visible_rows_for_height(&self, viewport_h: usize) -> usize {
        if viewport_h < 3 {
            return 1;
        }
        // Reserve: status(1) + help(1) = 2
        viewport_h.saturating_sub(2)
    }

    // -- Navigation --

    pub fn select_next(&mut self) {
        let total = self.total_items();
        if total > 0 && self.selected < total - 1 {
            self.selected += 1;
            self.clear_transient_status();
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
            self.clear_transient_status();
        }
    }

    pub fn select_first(&mut self) {
        self.selected = 0;
        self.clear_transient_status();
    }

    pub fn select_last(&mut self) {
        let total = self.total_items();
        if total > 0 {
            self.selected = total - 1;
        }
        self.clear_transient_status();
    }

    /// Jump to first item of the next group.
    pub fn jump_next_group(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let current_group = self.items[self.selected].group;
        for i in (self.selected + 1)..self.items.len() {
            if self.items[i].group != current_group {
                self.selected = i;
                self.clear_transient_status();
                return;
            }
        }
        self.selected = 0;
        self.clear_transient_status();
    }

    /// Jump to first item of the previous group.
    pub fn jump_prev_group(&mut self) {
        if self.items.is_empty() {
            return;
        }
        let current_group = self.items[self.selected].group;
        let group_start = self.items.iter().position(|it| it.group == current_group).unwrap_or(0);

        if group_start == 0 {
            let last_group = self.items.last().unwrap().group;
            self.selected = self.items.iter().position(|it| it.group == last_group).unwrap_or(0);
        } else {
            let prev_group = self.items[group_start - 1].group;
            self.selected = self.items.iter().position(|it| it.group == prev_group).unwrap_or(0);
        }
        self.clear_transient_status();
    }

    pub fn page_down(&mut self, page_size: usize) {
        let total = self.total_items();
        if total > 0 {
            self.selected = (self.selected + page_size).min(total - 1);
            self.clear_transient_status();
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        self.selected = self.selected.saturating_sub(page_size);
        self.clear_transient_status();
    }

    /// Adjust scroll_offset so the selected item is visible.
    pub fn ensure_visible(&mut self, viewport_h: usize) {
        let rows = self.rows();
        let sel_row = rows
            .iter()
            .position(|r| matches!(r, Row::Item(i) if *i == self.selected));
        let sel_row = match sel_row {
            Some(r) => r,
            None => return,
        };

        // Include group header above if first item in group
        let effective_row = if sel_row > 0 && matches!(rows[sel_row - 1], Row::GroupHeader(_)) {
            sel_row - 1
        } else {
            sel_row
        };

        let max_rows = self.visible_rows_for_height(viewport_h);

        if effective_row < self.scroll_offset {
            self.scroll_offset = effective_row;
        } else if sel_row >= self.scroll_offset + max_rows {
            self.scroll_offset = sel_row + 1 - max_rows;
        }
    }

    /// Reset selected item to its default value.
    pub fn reset_selected(&mut self) -> Option<ConfigAction> {
        let item = self.items.get(self.selected)?;
        let default = item.default_value.clone();
        let key = item.key.clone();
        let is_format = item.is_format;
        let is_repl = item.is_repl;

        if item.value == default {
            return None;
        }

        let send_value = default.trim_matches('\'').to_string();

        let item = &mut self.items[self.selected];
        item.value = default;
        item.source = if is_repl { "session" } else { "file" }.to_string();

        if is_repl {
            Some(ConfigAction::SetRepl { key, value: send_value })
        } else {
            Some(ConfigAction::SetValue {
                key,
                value: send_value,
                is_format,
            })
        }
    }

    fn clear_transient_status(&mut self) {
        if let Some((_, StatusKind::Success | StatusKind::Info)) = &self.status_message {
            self.status_message = None;
        }
    }

    // -- Item access --

    fn selected_item_mut(&mut self) -> Option<&mut ConfigItem> {
        self.items.get_mut(self.selected)
    }

    fn selected_item(&self) -> Option<&ConfigItem> {
        self.items.get(self.selected)
    }

    /// Toggle bool, cycle choice, or enter edit mode for int.
    pub fn toggle_or_enter_edit(&mut self) -> Option<ConfigAction> {
        if self.edit.is_some() {
            return self.confirm_edit();
        }

        let item = self.selected_item_mut()?;
        let is_format = item.is_format;
        let is_repl = item.is_repl;

        match &item.config_type {
            ConfigType::Bool => {
                let new_val = if item.value == "True" || item.value == "true" {
                    "false"
                } else {
                    "true"
                };
                item.value = new_val.to_string();
                item.source = if is_repl { "session" } else { "file" }.to_string();
                if is_repl {
                    Some(ConfigAction::SetRepl {
                        key: item.key.clone(),
                        value: new_val.to_string(),
                    })
                } else {
                    Some(ConfigAction::SetValue {
                        key: item.key.clone(),
                        value: new_val.to_string(),
                        is_format,
                    })
                }
            }
            ConfigType::Choice(choices) => {
                let current = item.value.trim_matches('\'').to_string();
                let idx = choices.iter().position(|c| *c == current).unwrap_or(0);
                let next = choices[(idx + 1) % choices.len()];
                item.value = format!("'{}'", next);
                item.source = if is_repl { "session" } else { "file" }.to_string();
                if is_repl {
                    Some(ConfigAction::SetRepl {
                        key: item.key.clone(),
                        value: next.to_string(),
                    })
                } else {
                    Some(ConfigAction::SetValue {
                        key: item.key.clone(),
                        value: next.to_string(),
                        is_format,
                    })
                }
            }
            ConfigType::Int { .. } => {
                let current = item.value.trim().to_string();
                let len = current.len();
                self.edit = Some(EditMode {
                    buffer: current,
                    cursor: len,
                });
                None
            }
        }
    }

    pub fn cancel_edit(&mut self) {
        self.edit = None;
        self.status_message = None;
    }

    /// Confirm the current edit. Returns action if valid.
    pub fn confirm_edit(&mut self) -> Option<ConfigAction> {
        let edit = self.edit.take()?;

        let (key, config_type, is_format, is_repl) = {
            let item = self.selected_item()?;
            (item.key.clone(), item.config_type.clone(), item.is_format, item.is_repl)
        };

        if let ConfigType::Int { min, max } = &config_type {
            match edit.buffer.parse::<i64>() {
                Ok(v) if v >= *min && v <= *max => {
                    if let Some(item) = self.selected_item_mut() {
                        item.value = v.to_string();
                        item.source = if is_repl { "session" } else { "file" }.to_string();
                    }
                    self.status_message = None;
                    if is_repl {
                        Some(ConfigAction::SetRepl {
                            key,
                            value: v.to_string(),
                        })
                    } else {
                        Some(ConfigAction::SetValue {
                            key,
                            value: v.to_string(),
                            is_format,
                        })
                    }
                }
                Ok(_) => {
                    self.status_message =
                        Some((format!("Value must be between {} and {}", min, max), StatusKind::Error));
                    self.edit = Some(edit);
                    None
                }
                Err(_) => {
                    self.status_message = Some(("Invalid number".to_string(), StatusKind::Error));
                    self.edit = Some(edit);
                    None
                }
            }
        } else {
            None
        }
    }

    // -- Edit mode key handling --

    pub fn edit_insert_char(&mut self, ch: char) {
        if let Some(ref mut edit) = self.edit {
            edit.buffer.insert(edit.cursor, ch);
            edit.cursor += 1;
        }
    }

    pub fn edit_backspace(&mut self) {
        if let Some(ref mut edit) = self.edit {
            if edit.cursor > 0 {
                edit.cursor -= 1;
                edit.buffer.remove(edit.cursor);
            }
        }
    }

    pub fn edit_move_left(&mut self) {
        if let Some(ref mut edit) = self.edit {
            if edit.cursor > 0 {
                edit.cursor -= 1;
            }
        }
    }

    pub fn edit_move_right(&mut self) {
        if let Some(ref mut edit) = self.edit {
            if edit.cursor < edit.buffer.len() {
                edit.cursor += 1;
            }
        }
    }
}

impl Default for ConfigEditorState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entries() -> Vec<(String, String, String)> {
        vec![
            ("executor".into(), "'vm'".into(), "default".into()),
            ("optimize".into(), "3".into(), "default".into()),
            ("tco".into(), "True".into(), "default".into()),
            ("jit".into(), "True".into(), "file".into()),
            ("no_color".into(), "False".into(), "default".into()),
            ("theme".into(), "'auto'".into(), "default".into()),
            ("enable_cache".into(), "True".into(), "default".into()),
            ("cache_max_size_mb".into(), "100".into(), "default".into()),
            ("cache_ttl_seconds".into(), "86400".into(), "default".into()),
            ("log_weird_errors".into(), "True".into(), "default".into()),
            ("max_weird_logs".into(), "50".into(), "default".into()),
            ("memory_limit".into(), "2048".into(), "default".into()),
            ("indent_size".into(), "4".into(), "default".into()),
            ("line_length".into(), "120".into(), "default".into()),
        ]
    }

    fn make_repl_entries() -> Vec<(String, String)> {
        vec![
            ("show_parse_time".into(), "false".into()),
            ("show_exec_time".into(), "false".into()),
            ("debug_mode".into(), "false".into()),
            ("max_history".into(), "1000".into()),
        ]
    }

    #[test]
    fn test_load_and_navigation() {
        let mut state = ConfigEditorState::new();
        state.load(make_entries(), make_repl_entries());

        assert!(state.active);
        assert_eq!(state.total_items(), 18); // 14 config + 4 repl
        assert_eq!(state.selected, 0);

        state.select_next();
        assert_eq!(state.selected, 1);

        state.select_prev();
        assert_eq!(state.selected, 0);

        state.select_prev();
        assert_eq!(state.selected, 0);

        for _ in 0..30 {
            state.select_next();
        }
        assert_eq!(state.selected, 17);
    }

    #[test]
    fn test_toggle_bool() {
        let mut state = ConfigEditorState::new();
        state.load(make_entries(), make_repl_entries());

        // tco is index 2
        state.selected = 2;
        let action = state.toggle_or_enter_edit();
        assert!(action.is_some());
        if let Some(ConfigAction::SetValue { key, value, is_format }) = action {
            assert_eq!(key, "tco");
            assert_eq!(value, "false");
            assert!(!is_format);
        }

        let action = state.toggle_or_enter_edit();
        if let Some(ConfigAction::SetValue { value, .. }) = action {
            assert_eq!(value, "true");
        }
    }

    #[test]
    fn test_toggle_repl_bool() {
        let mut state = ConfigEditorState::new();
        state.load(make_entries(), make_repl_entries());

        // show_parse_time is first in Repl group (index 14)
        state.selected = 14;
        assert_eq!(state.items[14].key, "show_parse_time");
        assert!(state.items[14].is_repl);

        let action = state.toggle_or_enter_edit();
        assert!(action.is_some());
        if let Some(ConfigAction::SetRepl { key, value }) = action {
            assert_eq!(key, "show_parse_time");
            assert_eq!(value, "true");
        }
    }

    #[test]
    fn test_cycle_choice() {
        let mut state = ConfigEditorState::new();
        state.load(make_entries(), make_repl_entries());

        state.selected = 0;
        let action = state.toggle_or_enter_edit();
        if let Some(ConfigAction::SetValue { key, value, .. }) = action {
            assert_eq!(key, "executor");
            assert_eq!(value, "ast");
        }

        let action = state.toggle_or_enter_edit();
        if let Some(ConfigAction::SetValue { value, .. }) = action {
            assert_eq!(value, "vm");
        }
    }

    #[test]
    fn test_edit_int() {
        let mut state = ConfigEditorState::new();
        state.load(make_entries(), make_repl_entries());

        state.selected = 1; // optimize
        let action = state.toggle_or_enter_edit();
        assert!(action.is_none());
        assert!(state.edit.is_some());

        state.edit = Some(EditMode {
            buffer: "2".to_string(),
            cursor: 1,
        });
        let action = state.confirm_edit();
        assert!(action.is_some());
        if let Some(ConfigAction::SetValue { value, .. }) = action {
            assert_eq!(value, "2");
        }
    }

    #[test]
    fn test_edit_int_validation() {
        let mut state = ConfigEditorState::new();
        state.load(make_entries(), make_repl_entries());

        state.selected = 1; // optimize, 0..3
        state.toggle_or_enter_edit();

        state.edit = Some(EditMode {
            buffer: "5".to_string(),
            cursor: 1,
        });
        let action = state.confirm_edit();
        assert!(action.is_none());
        assert!(state.status_message.is_some());
        assert!(state.edit.is_some());
    }

    #[test]
    fn test_format_key_is_format() {
        let mut state = ConfigEditorState::new();
        state.load(make_entries(), make_repl_entries());

        // indent_size is index 12 (4 exec + 2 display + 3 cache + 3 debug = 12)
        state.selected = 12;
        assert!(state.items[12].is_format);
        assert_eq!(state.items[12].key, "indent_size");

        state.selected = 0;
        assert!(!state.items[0].is_format);
    }

    #[test]
    fn test_visible_lines() {
        let mut state = ConfigEditorState::new();
        assert_eq!(state.visible_lines(), 0);

        state.load(make_entries(), make_repl_entries());
        assert!(state.visible_lines() > 0);
    }

    #[test]
    fn test_reset_state() {
        let mut state = ConfigEditorState::new();
        state.load(make_entries(), make_repl_entries());

        state.reset();
        assert!(!state.active);
        assert!(state.items.is_empty());
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn test_group_navigation() {
        let mut state = ConfigEditorState::new();
        state.load(make_entries(), make_repl_entries());

        assert_eq!(state.selected, 0);
        assert_eq!(state.items[0].group, ConfigGroup::Execution);

        state.jump_next_group();
        assert_eq!(state.items[state.selected].group, ConfigGroup::Display);

        state.jump_next_group();
        assert_eq!(state.items[state.selected].group, ConfigGroup::Cache);

        state.jump_prev_group();
        assert_eq!(state.items[state.selected].group, ConfigGroup::Display);

        state.jump_prev_group();
        assert_eq!(state.items[state.selected].group, ConfigGroup::Execution);

        // Wrap to last group (Repl)
        state.jump_prev_group();
        assert_eq!(state.items[state.selected].group, ConfigGroup::Repl);
    }

    #[test]
    fn test_scroll_ensure_visible() {
        let mut state = ConfigEditorState::new();
        state.load(make_entries(), make_repl_entries());

        state.selected = 17; // last item
        state.ensure_visible(9);
        assert!(state.scroll_offset > 0);

        state.selected = 0;
        state.ensure_visible(9);
        assert_eq!(state.scroll_offset, 0);
    }

    #[test]
    fn test_reset_to_default() {
        let mut state = ConfigEditorState::new();
        state.load(make_entries(), make_repl_entries());

        // jit is at index 3, value "True", default "False"
        state.selected = 3;
        assert_eq!(state.items[3].key, "jit");
        assert!(state.items[3].is_modified());

        let action = state.reset_selected();
        assert!(action.is_some());
        if let Some(ConfigAction::SetValue { key, value, .. }) = action {
            assert_eq!(key, "jit");
            assert_eq!(value, "False");
        }
        assert!(!state.items[3].is_modified());

        let action = state.reset_selected();
        assert!(action.is_none());
    }

    #[test]
    fn test_rows_include_headers() {
        let mut state = ConfigEditorState::new();
        state.load(make_entries(), make_repl_entries());

        let rows = state.rows();

        let header_count = rows.iter().filter(|r| matches!(r, Row::GroupHeader(_))).count();
        assert_eq!(header_count, 6); // 6 groups now

        assert!(matches!(rows[0], Row::GroupHeader(_)));

        let item_count = rows.iter().filter(|r| matches!(r, Row::Item(_))).count();
        assert_eq!(item_count, 18);
    }

    #[test]
    fn test_items_ordered_by_group() {
        let mut state = ConfigEditorState::new();
        state.load(make_entries(), make_repl_entries());

        let groups: Vec<ConfigGroup> = state.items.iter().map(|it| it.group).collect();
        let mut seen_groups = Vec::new();
        for g in &groups {
            if seen_groups.last() != Some(g) {
                seen_groups.push(*g);
            }
        }
        assert_eq!(
            seen_groups,
            vec![
                ConfigGroup::Execution,
                ConfigGroup::Display,
                ConfigGroup::Cache,
                ConfigGroup::Debug,
                ConfigGroup::Format,
                ConfigGroup::Repl,
            ]
        );
    }
}
