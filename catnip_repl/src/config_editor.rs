// FILE: catnip_repl/src/config_editor.rs
//! Interactive config editor overlay for the REPL.

/// Type constraint for a config key.
#[derive(Debug, Clone)]
pub enum ConfigType {
    Bool,
    Int { min: i64, max: i64 },
    Choice(&'static [&'static str]),
}

/// One item in the config editor list.
#[derive(Debug, Clone)]
pub struct ConfigItem {
    pub key: String,
    pub value: String,
    pub source: String,
    pub config_type: ConfigType,
}

/// Inline edit mode for Int/Choice values.
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
}

/// Config key metadata (type constraints).
struct KeyMeta {
    key: &'static str,
    config_type: ConfigType,
}

const CONFIG_KEYS: &[KeyMeta] = &[
    KeyMeta {
        key: "no_color",
        config_type: ConfigType::Bool,
    },
    KeyMeta {
        key: "enable_cache",
        config_type: ConfigType::Bool,
    },
    KeyMeta {
        key: "jit",
        config_type: ConfigType::Bool,
    },
    KeyMeta {
        key: "tco",
        config_type: ConfigType::Bool,
    },
    KeyMeta {
        key: "log_weird_errors",
        config_type: ConfigType::Bool,
    },
    KeyMeta {
        key: "optimize",
        config_type: ConfigType::Int { min: 0, max: 3 },
    },
    KeyMeta {
        key: "executor",
        config_type: ConfigType::Choice(&["vm", "ast"]),
    },
    KeyMeta {
        key: "theme",
        config_type: ConfigType::Choice(&["auto", "dark", "light"]),
    },
    KeyMeta {
        key: "memory_limit",
        config_type: ConfigType::Int { min: 0, max: 65536 },
    },
    KeyMeta {
        key: "cache_max_size_mb",
        config_type: ConfigType::Int { min: 0, max: 10000 },
    },
    KeyMeta {
        key: "cache_ttl_seconds",
        config_type: ConfigType::Int {
            min: 0,
            max: 999999,
        },
    },
    KeyMeta {
        key: "max_weird_logs",
        config_type: ConfigType::Int { min: 0, max: 10000 },
    },
];

const FORMAT_KEYS: &[KeyMeta] = &[
    KeyMeta {
        key: "indent_size",
        config_type: ConfigType::Int { min: 1, max: 16 },
    },
    KeyMeta {
        key: "line_length",
        config_type: ConfigType::Int { min: 40, max: 500 },
    },
];

/// Interactive config editor state.
pub struct ConfigEditorState {
    pub active: bool,
    pub items: Vec<ConfigItem>,
    pub format_items: Vec<ConfigItem>,
    pub selected: usize,
    pub edit: Option<EditMode>,
    pub status_message: Option<String>,
}

impl ConfigEditorState {
    pub fn new() -> Self {
        Self {
            active: false,
            items: Vec::new(),
            format_items: Vec::new(),
            selected: 0,
            edit: None,
            status_message: None,
        }
    }

    pub fn reset(&mut self) {
        self.active = false;
        self.items.clear();
        self.format_items.clear();
        self.selected = 0;
        self.edit = None;
        self.status_message = None;
    }

    /// Populate items from ConfigManager data.
    /// `entries`: Vec<(key, value_repr, source_str)> for main keys.
    /// `format_entries`: same for format.* keys.
    pub fn load(
        &mut self,
        entries: Vec<(String, String, String)>,
        format_entries: Vec<(String, String, String)>,
    ) {
        self.items.clear();
        self.format_items.clear();
        self.selected = 0;
        self.edit = None;
        self.status_message = None;

        for (key, value, source) in entries {
            let config_type = CONFIG_KEYS
                .iter()
                .find(|m| m.key == key)
                .map(|m| m.config_type.clone())
                .unwrap_or(ConfigType::Bool);
            self.items.push(ConfigItem {
                key,
                value,
                source,
                config_type,
            });
        }

        for (key, value, source) in format_entries {
            let config_type = FORMAT_KEYS
                .iter()
                .find(|m| m.key == key)
                .map(|m| m.config_type.clone())
                .unwrap_or(ConfigType::Int { min: 0, max: 9999 });
            self.format_items.push(ConfigItem {
                key,
                value,
                source,
                config_type,
            });
        }

        self.active = true;
    }

    pub fn total_items(&self) -> usize {
        self.items.len() + self.format_items.len()
    }

    /// Height of the overlay (header + separator + items + format separator + format items + help line).
    pub fn visible_lines(&self) -> usize {
        if !self.active || self.total_items() == 0 {
            return 0;
        }
        // header + separator + main items + format separator + format items + status/help
        2 + self.items.len() + 1 + self.format_items.len() + 1
    }

    pub fn select_next(&mut self) {
        let total = self.total_items();
        if total > 0 && self.selected < total - 1 {
            self.selected += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Get the currently selected item (mutable).
    fn selected_item_mut(&mut self) -> Option<&mut ConfigItem> {
        let n = self.items.len();
        if self.selected < n {
            Some(&mut self.items[self.selected])
        } else {
            let fi = self.selected - n;
            self.format_items.get_mut(fi)
        }
    }

    /// Get the currently selected item (immutable).
    fn selected_item(&self) -> Option<&ConfigItem> {
        let n = self.items.len();
        if self.selected < n {
            Some(&self.items[self.selected])
        } else {
            let fi = self.selected - n;
            self.format_items.get(fi)
        }
    }

    /// Whether the selected item is a format key.
    fn selected_is_format(&self) -> bool {
        self.selected >= self.items.len()
    }

    /// Toggle bool, cycle choice, or enter edit mode for int.
    /// Returns a ConfigAction if value was changed immediately (bool/choice).
    pub fn toggle_or_enter_edit(&mut self) -> Option<ConfigAction> {
        if self.edit.is_some() {
            return self.confirm_edit();
        }

        let is_format = self.selected_is_format();
        let item = self.selected_item_mut()?;

        match &item.config_type {
            ConfigType::Bool => {
                let new_val = if item.value == "True" || item.value == "true" {
                    "false"
                } else {
                    "true"
                };
                item.value = new_val.to_string();
                item.source = "file".to_string();
                Some(ConfigAction::SetValue {
                    key: item.key.clone(),
                    value: new_val.to_string(),
                    is_format,
                })
            }
            ConfigType::Choice(choices) => {
                let current = item.value.trim_matches('\'').to_string();
                let idx = choices.iter().position(|c| *c == current).unwrap_or(0);
                let next = choices[(idx + 1) % choices.len()];
                item.value = format!("'{}'", next);
                item.source = "file".to_string();
                Some(ConfigAction::SetValue {
                    key: item.key.clone(),
                    value: next.to_string(),
                    is_format,
                })
            }
            ConfigType::Int { .. } => {
                // Enter edit mode with current value
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
        let is_format = self.selected_is_format();

        // Read item info without holding a mutable borrow
        let (key, config_type) = {
            let item = self.selected_item()?;
            (item.key.clone(), item.config_type.clone())
        };

        if let ConfigType::Int { min, max } = &config_type {
            match edit.buffer.parse::<i64>() {
                Ok(v) if v >= *min && v <= *max => {
                    if let Some(item) = self.selected_item_mut() {
                        item.value = v.to_string();
                        item.source = "file".to_string();
                    }
                    self.status_message = None;
                    Some(ConfigAction::SetValue {
                        key,
                        value: v.to_string(),
                        is_format,
                    })
                }
                Ok(_) => {
                    self.status_message =
                        Some(format!("Value must be between {} and {}", min, max));
                    self.edit = Some(edit);
                    None
                }
                Err(_) => {
                    self.status_message = Some("Invalid number".to_string());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entries() -> (Vec<(String, String, String)>, Vec<(String, String, String)>) {
        let main = vec![
            ("no_color".into(), "False".into(), "default".into()),
            ("jit".into(), "True".into(), "file".into()),
            ("tco".into(), "True".into(), "default".into()),
            ("optimize".into(), "3".into(), "default".into()),
            ("executor".into(), "'vm'".into(), "default".into()),
        ];
        let format = vec![
            ("indent_size".into(), "4".into(), "default".into()),
            ("line_length".into(), "120".into(), "default".into()),
        ];
        (main, format)
    }

    #[test]
    fn test_load_and_navigation() {
        let mut state = ConfigEditorState::new();
        let (main, fmt) = make_entries();
        state.load(main, fmt);

        assert!(state.active);
        assert_eq!(state.total_items(), 7);
        assert_eq!(state.selected, 0);

        state.select_next();
        assert_eq!(state.selected, 1);

        state.select_prev();
        assert_eq!(state.selected, 0);

        // Can't go below 0
        state.select_prev();
        assert_eq!(state.selected, 0);

        // Go to last
        for _ in 0..10 {
            state.select_next();
        }
        assert_eq!(state.selected, 6);
    }

    #[test]
    fn test_toggle_bool() {
        let mut state = ConfigEditorState::new();
        let (main, fmt) = make_entries();
        state.load(main, fmt);

        // First item is no_color = False
        let action = state.toggle_or_enter_edit();
        assert!(action.is_some());
        if let Some(ConfigAction::SetValue {
            key,
            value,
            is_format,
        }) = action
        {
            assert_eq!(key, "no_color");
            assert_eq!(value, "true");
            assert!(!is_format);
        }
        assert_eq!(state.items[0].value, "true");

        // Toggle back
        let action = state.toggle_or_enter_edit();
        if let Some(ConfigAction::SetValue { value, .. }) = action {
            assert_eq!(value, "false");
        }
    }

    #[test]
    fn test_cycle_choice() {
        let mut state = ConfigEditorState::new();
        let (main, fmt) = make_entries();
        state.load(main, fmt);

        // Select executor (index 4)
        state.selected = 4;
        let action = state.toggle_or_enter_edit();
        if let Some(ConfigAction::SetValue { key, value, .. }) = action {
            assert_eq!(key, "executor");
            assert_eq!(value, "ast"); // vm -> ast
        }

        let action = state.toggle_or_enter_edit();
        if let Some(ConfigAction::SetValue { value, .. }) = action {
            assert_eq!(value, "vm"); // ast -> vm (wraps)
        }
    }

    #[test]
    fn test_edit_int() {
        let mut state = ConfigEditorState::new();
        let (main, fmt) = make_entries();
        state.load(main, fmt);

        // Select optimize (index 3), type Int { 0..3 }
        state.selected = 3;
        let action = state.toggle_or_enter_edit();
        assert!(action.is_none()); // enters edit mode
        assert!(state.edit.is_some());

        // Clear and type "2"
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
        let (main, fmt) = make_entries();
        state.load(main, fmt);

        state.selected = 3; // optimize, 0..3
        state.toggle_or_enter_edit(); // enter edit

        state.edit = Some(EditMode {
            buffer: "5".to_string(),
            cursor: 1,
        });
        let action = state.confirm_edit();
        assert!(action.is_none()); // rejected
        assert!(state.status_message.is_some());
        assert!(state.edit.is_some()); // edit still active
    }

    #[test]
    fn test_format_key_is_format() {
        let mut state = ConfigEditorState::new();
        let (main, fmt) = make_entries();
        state.load(main, fmt);

        // index 5 = first format item (indent_size)
        state.selected = 5;
        assert!(state.selected_is_format());

        state.selected = 0;
        assert!(!state.selected_is_format());
    }

    #[test]
    fn test_visible_lines() {
        let mut state = ConfigEditorState::new();
        assert_eq!(state.visible_lines(), 0);

        let (main, fmt) = make_entries();
        state.load(main, fmt);
        // 2 (header+sep) + 5 (main) + 1 (format sep) + 2 (format) + 1 (help) = 11
        assert_eq!(state.visible_lines(), 11);
    }

    #[test]
    fn test_reset() {
        let mut state = ConfigEditorState::new();
        let (main, fmt) = make_entries();
        state.load(main, fmt);

        state.reset();
        assert!(!state.active);
        assert!(state.items.is_empty());
        assert_eq!(state.selected, 0);
    }
}
