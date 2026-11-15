//! Small input widgets with no rendering of their own, so they are easy to test.

use zeroize::Zeroizing;

/// A single-line text field. Contents are wiped from memory when dropped.
#[derive(Default)]
pub struct TextInput {
    value: Zeroizing<String>,
    pub masked: bool,
}

impl TextInput {
    pub fn new(initial: &str) -> Self {
        Self {
            value: Zeroizing::new(initial.to_owned()),
            masked: false,
        }
    }

    pub fn masked() -> Self {
        Self {
            masked: true,
            ..Self::default()
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn is_empty(&self) -> bool {
        self.value.is_empty()
    }

    pub fn push(&mut self, c: char) {
        self.value.push(c);
    }

    pub fn pop(&mut self) {
        self.value.pop();
    }

    pub fn clear(&mut self) {
        self.value.clear();
    }

    /// What to draw: the text, or one dot per character for secrets.
    pub fn display(&self) -> String {
        if self.masked {
            "*".repeat(self.value.chars().count())
        } else {
            self.value.to_string()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// What the app stores when this is chosen.
    pub value: String,
    /// What the user sees and filters on.
    pub label: String,
    /// Shown dimmed next to the label and also searched, for example a code.
    pub detail: String,
}

impl Item {
    pub fn new(value: &str, label: &str) -> Self {
        Self::with_detail(value, label, "")
    }

    pub fn with_detail(value: &str, label: &str, detail: &str) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            detail: detail.into(),
        }
    }
}

/// A list narrowed by typing. Matching is case-insensitive on label and detail.
#[derive(Debug, Default)]
pub struct FilterList {
    items: Vec<Item>,
    filter: String,
    /// Index into the *visible* items.
    selected: usize,
}

impl FilterList {
    pub fn new(items: Vec<Item>) -> Self {
        Self {
            items,
            ..Self::default()
        }
    }

    pub fn total(&self) -> usize {
        self.items.len()
    }

    pub fn filter(&self) -> &str {
        &self.filter
    }

    pub fn visible(&self) -> Vec<&Item> {
        let needle = self.filter.to_lowercase();
        self.items
            .iter()
            .filter(|i| {
                i.label.to_lowercase().contains(&needle)
                    || i.detail.to_lowercase().contains(&needle)
            })
            .collect()
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn selected(&self) -> Option<&Item> {
        self.visible().get(self.selected).copied()
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn down(&mut self) {
        let last = self.visible().len().saturating_sub(1);
        self.selected = (self.selected + 1).min(last);
    }

    pub fn page(&mut self, delta: isize) {
        let last = self.visible().len().saturating_sub(1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }

    pub fn push_filter(&mut self, c: char) {
        self.filter.push(c);
        self.selected = 0;
    }

    pub fn pop_filter(&mut self) {
        self.filter.pop();
        self.selected = 0;
    }

    pub fn clear_filter(&mut self) {
        self.filter.clear();
        self.selected = 0;
    }

    /// Clear the filter and select the item with this value, if present.
    pub fn select_value(&mut self, value: &str) {
        self.filter.clear();
        self.selected = self
            .items
            .iter()
            .position(|i| i.value == value)
            .unwrap_or(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> FilterList {
        FilterList::new(vec![
            Item::new("de", "de - German"),
            Item::new("ch", "ch - German (Switzerland)"),
            Item::new("us", "us - English (US)"),
        ])
    }

    #[test]
    fn filters_case_insensitively_on_label() {
        let mut l = list();
        for c in "GERMAN".chars() {
            l.push_filter(c);
        }
        let v: Vec<_> = l.visible().iter().map(|i| i.value.as_str()).collect();
        assert_eq!(v, ["de", "ch"]);
        l.pop_filter();
        assert_eq!(l.visible().len(), 2);
    }

    #[test]
    fn detail_is_searchable_too() {
        let mut l = FilterList::new(vec![
            Item::with_detail("ch", "German (Switzerland)", "ch"),
            Item::with_detail("us", "English (US)", "us"),
        ]);
        l.push_filter('u');
        l.push_filter('s');
        let v: Vec<_> = l.visible().iter().map(|i| i.value.as_str()).collect();
        assert_eq!(v, ["us"]);
        l.clear_filter();
        for c in "ch".chars() {
            l.push_filter(c);
        }
        assert_eq!(l.visible().len(), 1);
    }

    #[test]
    fn selection_moves_and_clamps() {
        let mut l = list();
        l.up();
        assert_eq!(l.selected().unwrap().value, "de");
        l.down();
        l.down();
        l.down();
        assert_eq!(l.selected().unwrap().value, "us");
        l.page(-10);
        assert_eq!(l.selected().unwrap().value, "de");
    }

    #[test]
    fn typing_resets_selection_and_can_empty_the_list() {
        let mut l = list();
        l.down();
        l.push_filter('q');
        assert!(l.selected().is_none());
        l.down();
        l.up();
        l.clear_filter();
        assert_eq!(l.selected().unwrap().value, "de");
    }

    #[test]
    fn select_value_clears_filter() {
        let mut l = list();
        l.push_filter('u');
        l.select_value("us");
        assert_eq!(l.filter(), "");
        assert_eq!(l.selected().unwrap().value, "us");
    }

    #[test]
    fn masked_input_hides_text() {
        let mut t = TextInput::masked();
        t.push('a');
        t.push('é');
        assert_eq!(t.display(), "**");
        assert_eq!(t.value(), "aé");
        t.pop();
        assert_eq!(t.value(), "a");
    }
}
