//! The screens that choose from a filterable list: timezone, language and
//! keyboard.

use super::{list_edit, Errors, Flow, Target};
use crate::widgets::{FilterList, Item};
use crossterm::event::{KeyCode, KeyEvent};
use installer_core::{check_keyboard_layout, check_keyboard_variant, Catalog};

const NOTHING_MATCHES: &str = "nothing matches, edit the filter";

fn selected_value(list: &FilterList) -> String {
    list.selected().map(|i| i.value.clone()).unwrap_or_default()
}

/// One list, one choice: timezone and language.
pub struct Picker {
    list: FilterList,
    errors: Errors,
}

impl Picker {
    pub fn new(values: &[String], default: &str) -> Self {
        let mut list = FilterList::new(values.iter().map(|s| Item::new(s, s)).collect());
        list.select_value(default);
        Self {
            list,
            errors: Errors::default(),
        }
    }

    pub fn list(&self) -> &FilterList {
        &self.list
    }

    pub fn value(&self) -> String {
        selected_value(&self.list)
    }

    pub fn select(&mut self, value: &str) {
        self.list.select_value(value);
    }

    pub fn errors(&self) -> &Errors {
        &self.errors
    }

    pub fn key(&mut self, key: &KeyEvent) -> Flow {
        self.errors.clear();
        if list_edit(&mut self.list, key) {
            return Flow::Stay;
        }
        match key.code {
            KeyCode::Esc if self.list.filter().is_empty() => Flow::Back,
            KeyCode::Esc => {
                self.list.clear_filter();
                Flow::Stay
            }
            KeyCode::Enter if self.list.selected().is_none() => {
                self.errors.set(Target::Screen, NOTHING_MATCHES);
                Flow::Stay
            }
            KeyCode::Enter => Flow::Next,
            _ => Flow::Stay,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Layout,
    Variant,
}

/// Keyboard layout on the left, its variant on the right.
pub struct KeyboardPicker {
    layout: FilterList,
    variant: FilterList,
    pane: Pane,
    /// Layout the variant list was built for.
    variant_layout: String,
    errors: Errors,
}

impl KeyboardPicker {
    pub fn new(catalog: &Catalog) -> Self {
        let mut layout = FilterList::new(
            catalog
                .layouts
                .iter()
                .map(|l| Item::with_detail(&l.name, &l.description, &l.name))
                .collect(),
        );
        layout.select_value("us");
        let mut picker = Self {
            layout,
            variant: FilterList::default(),
            pane: Pane::Layout,
            variant_layout: String::new(),
            errors: Errors::default(),
        };
        picker.sync_variants(catalog);
        picker
    }

    pub fn layout_list(&self) -> &FilterList {
        &self.layout
    }

    pub fn variant_list(&self) -> &FilterList {
        &self.variant
    }

    pub fn pane(&self) -> Pane {
        self.pane
    }

    pub fn errors(&self) -> &Errors {
        &self.errors
    }

    pub fn layout_value(&self) -> String {
        selected_value(&self.layout)
    }

    pub fn variant_value(&self) -> Option<String> {
        Some(selected_value(&self.variant)).filter(|v| !v.is_empty())
    }

    /// Description of the highlighted layout, for the variant pane's title.
    pub fn layout_description(&self) -> String {
        self.layout
            .selected()
            .map(|i| i.label.clone())
            .unwrap_or_default()
    }

    fn has_variants(&self, catalog: &Catalog) -> bool {
        catalog
            .layout(&self.layout_value())
            .is_some_and(|l| !l.variants.is_empty())
    }

    /// Keep the variant pane in step with the highlighted layout.
    fn sync_variants(&mut self, catalog: &Catalog) {
        let layout = self.layout_value();
        if layout == self.variant_layout {
            return;
        }
        let mut items = vec![Item::with_detail("", "(default)", "")];
        if let Some(l) = catalog.layout(&layout) {
            items.extend(
                l.variants
                    .iter()
                    .map(|v| Item::with_detail(&v.name, &v.description, &v.name)),
            );
        }
        self.variant = FilterList::new(items);
        self.variant_layout = layout;
    }

    fn active(&mut self) -> &mut FilterList {
        match self.pane {
            Pane::Layout => &mut self.layout,
            Pane::Variant => &mut self.variant,
        }
    }

    /// Next means the choice was confirmed and is valid.
    pub fn key(&mut self, key: &KeyEvent, catalog: &Catalog) -> Flow {
        self.errors.clear();
        match key.code {
            KeyCode::Tab => {
                self.pane = match self.pane {
                    Pane::Layout => Pane::Variant,
                    Pane::Variant => Pane::Layout,
                };
            }
            KeyCode::Left => self.pane = Pane::Layout,
            KeyCode::Right => self.pane = Pane::Variant,
            KeyCode::Esc => {
                let pane = self.pane;
                let list = self.active();
                if !list.filter().is_empty() {
                    list.clear_filter();
                    if pane == Pane::Layout {
                        self.sync_variants(catalog);
                    }
                } else if pane == Pane::Variant {
                    self.pane = Pane::Layout;
                } else {
                    return Flow::Back;
                }
            }
            KeyCode::Enter => {
                if self.layout.selected().is_none() {
                    self.errors.set(Target::Screen, NOTHING_MATCHES);
                } else if self.pane == Pane::Layout && self.has_variants(catalog) {
                    // Let the user pick a variant; Enter there confirms.
                    self.pane = Pane::Variant;
                } else {
                    return self.confirm(catalog);
                }
            }
            _ => {
                let pane = self.pane;
                if list_edit(self.active(), key) && pane == Pane::Layout {
                    self.sync_variants(catalog);
                }
            }
        }
        Flow::Stay
    }

    fn confirm(&mut self, catalog: &Catalog) -> Flow {
        let layout = self.layout_value();
        let checked = check_keyboard_layout(catalog, &layout).and_then(|()| {
            self.variant_value()
                .map_or(Ok(()), |v| check_keyboard_variant(catalog, &layout, &v))
        });
        match checked {
            Ok(()) => Flow::Next,
            Err(message) => {
                self.errors.set(Target::Screen, message);
                Flow::Stay
            }
        }
    }
}
