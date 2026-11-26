//! Settings tab: a sidebar plus one scrollable pane. Probes and presets are
//! edited through dedicated forms; the option sections are a flat field list.

mod form;
mod input;
mod view;

#[cfg(test)]
pub(crate) use form::field_labels;
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
enum EditTarget {
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
    edit_target: EditTarget,
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
