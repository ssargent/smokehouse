//! Settings tab: a sidebar plus one scrollable pane. Probes and presets are
//! edited through dedicated forms; the option sections are a flat field list.

mod form;
mod input;
mod view;

#[cfg(test)]
pub(crate) use form::field_labels;
pub(crate) use input::footer_hint;
pub(crate) use input::handle_key;
pub(crate) use view::draw;

use std::cell::RefCell;

use ratatui::widgets::ListState;

/// Sidebar sections, in fixed order.
const SECTIONS: usize = 7;
/// Sidebar labels, matched to [`SECTIONS`].
const SECTION_TITLES: [&str; SECTIONS] = [
    "Probes",
    "Presets",
    "Alarms",
    "Speech",
    "Cook & estimates",
    "Display",
    "Device & data",
];
/// Width of the sidebar.
const SIDEBAR_WIDTH: u16 = 22;
/// Width of the overlay dialogs.
const OVERLAY_WIDTH: u16 = 62;

/// Which field the text editor writes back to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum EditTarget {
    #[default]
    None,
    OptionField,
    ProbeLabel,
    PresetName,
}

/// Focus and overlay state for the Settings tab.
#[derive(Debug, Default)]
pub(crate) struct SettingsUi {
    /// Focused sidebar section.
    pub(crate) section: usize,
    /// Focused field within the pane (option sections).
    pub(crate) field: usize,
    /// Probe row of the read-only table, and the probe a form edits.
    pub(crate) probe_row: usize,
    /// Cursor over the preset list (the last row is "＋ New preset").
    pub(crate) preset_index: usize,
    pub(crate) editing: bool,
    pub(crate) edit_buffer: String,
    pub(crate) edit_cursor: usize,
    /// When a pre-filled value is being edited, the first typed key replaces it.
    edit_replace: bool,
    /// Validation message shown while an edit is rejected.
    pub(crate) edit_error: Option<String>,
    edit_target: EditTarget,
    /// Whether the Forget action is awaiting confirmation.
    pub(crate) confirm_forget: bool,
    /// Open probe form, if any.
    probe_form: Option<ProbeForm>,
    /// Open preset form, if any.
    preset_form: Option<PresetForm>,
    /// Open picker, if any.
    picker: Option<Picker>,
    pub(crate) confirm_delete: Option<usize>,
    /// Scroll state for the option pane.
    list_state: RefCell<ListState>,
}

impl SettingsUi {
    /// Whether any overlay (popup or text edit) is open.
    #[must_use]
    pub(crate) fn overlay_active(&self) -> bool {
        self.editing
            || self.probe_form.is_some()
            || self.preset_form.is_some()
            || self.picker.is_some()
            || self.confirm_delete.is_some()
            || self.confirm_forget
    }

    fn edit_limit(&self) -> usize {
        match self.edit_target {
            EditTarget::ProbeLabel => 18,
            EditTarget::PresetName => 40,
            _ => 64,
        }
    }

    /// Clear any validation message.
    pub(crate) fn clear_edit_error(&mut self) {
        self.edit_error = None;
    }

    fn edit_len(&self) -> usize {
        self.edit_buffer.chars().count()
    }

    /// Byte offset of the cursor.
    fn cursor_byte(&self) -> usize {
        self.edit_buffer
            .char_indices()
            .nth(self.edit_cursor)
            .map_or(self.edit_buffer.len(), |(index, _)| index)
    }

    /// Start editing `text` with the cursor at the end.
    pub(crate) fn edit_start(&mut self, text: String) {
        self.edit_replace = !text.is_empty();
        self.editing = true;
        self.edit_cursor = text.chars().count();
        self.edit_buffer = text;
    }

    /// Stop editing.
    pub(crate) fn edit_stop(&mut self) {
        self.editing = false;
        self.edit_buffer.clear();
        self.edit_cursor = 0;
        self.edit_target = EditTarget::None;
    }

    /// Insert a character, replacing a pre-filled value on the first key.
    pub(crate) fn edit_insert(&mut self, character: char) {
        if self.edit_replace {
            self.edit_replace = false;
            self.edit_buffer.clear();
            self.edit_cursor = 0;
        }
        if self.edit_len() >= self.edit_limit() {
            return;
        }
        let byte = self.cursor_byte();
        self.edit_buffer.insert(byte, character);
        self.edit_cursor += 1;
    }

    /// Delete the character before the cursor.
    pub(crate) fn edit_backspace(&mut self) {
        if self.edit_cursor == 0 {
            return;
        }
        let end = self.cursor_byte();
        let start = self.edit_buffer[..end]
            .char_indices()
            .next_back()
            .map_or(0, |(index, _)| index);
        self.edit_buffer.replace_range(start..end, "");
        self.edit_cursor -= 1;
    }

    /// Delete the character under the cursor.
    pub(crate) fn edit_delete(&mut self) {
        let start = self.cursor_byte();
        if start >= self.edit_buffer.len() {
            return;
        }
        let end = self.edit_buffer[start..]
            .char_indices()
            .nth(1)
            .map_or(self.edit_buffer.len(), |(index, _)| start + index);
        self.edit_buffer.replace_range(start..end, "");
    }

    /// Clear the buffer.
    pub(crate) fn edit_clear(&mut self) {
        self.edit_buffer.clear();
        self.edit_cursor = 0;
    }

    /// Move the cursor by `delta` characters.
    pub(crate) fn edit_move(&mut self, delta: i32) {
        let len = self.edit_len() as i32;
        self.edit_cursor = (self.edit_cursor as i32 + delta).clamp(0, len) as usize;
    }

    /// Put the cursor at the start.
    pub(crate) fn edit_home(&mut self) {
        self.edit_cursor = 0;
    }

    /// Put the cursor at the end.
    pub(crate) fn edit_end(&mut self) {
        self.edit_cursor = self.edit_len();
    }

    /// The buffer with a cursor marker and a `used/limit` counter.
    #[must_use]
    pub(crate) fn edit_display(&self) -> String {
        let mut out = String::new();
        for (index, character) in self.edit_buffer.chars().enumerate() {
            if index == self.edit_cursor {
                out.push('▏');
            }
            out.push(character);
        }
        if self.edit_cursor >= self.edit_len() {
            out.push('▏');
        }
        format!("{out} {}/{}", self.edit_len(), self.edit_limit())
    }

    /// The current edit target.
    pub(super) fn edit_target(&self) -> EditTarget {
        self.edit_target
    }

    /// Set the edit target.
    pub(super) fn set_edit_target(&mut self, target: EditTarget) {
        self.edit_target = target;
    }
}

/// The open probe form.
#[derive(Clone, Copy, Debug)]
struct ProbeForm {
    probe: usize,
    field: usize,
}

/// The open preset form.
#[derive(Clone, Copy, Debug)]
struct PresetForm {
    index: usize,
    field: usize,
    /// A freshly-created preset whose name is being typed.
    new: bool,
}

/// The open picker overlay.
#[derive(Clone, Debug)]
struct Picker {
    target: PickerTarget,
    index: usize,
    items: Vec<String>,
}

/// What a picker selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PickerTarget {
    ProbePreset,
    Voice,
    AudioDevice,
}
