//! The catalogue screen's cursor (retail's `S+0x50`: the super category, sub category and item indices, whether the
//! sub category list or the item list shows (`S+0x2CF`), the item index remembered per sub category (`C+0x34`)), moved
//! by the catalogue keys of the park editor (`825CAC18`) and the open-world dropper (`825C6C68`), docs/object-dropper-
//! survey.md "The screen". It walks a [`super::Catalog::listing`].

/// (super category, [(sub category, item ids)]) in the tree's order (`Catalog::listing`).
pub type Listing = Vec<(String, Vec<(String, Vec<String>)>)>;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Browse {
    pub listing: Listing,
    pub super_index: usize,
    pub sub_index: usize,
    pub item_index: usize,
    /// Each sub category's item index, kept while the super category stays.
    remembered: Vec<usize>,
    /// The sub category list shows (the item list once A opened a sub category).
    pub subs: bool,
}

impl Browse {
    pub fn new(listing: Listing) -> Self {
        let mut browse = Self { listing, subs: true, ..Default::default() };
        browse.remembered = vec![0; browse.sub_count()];
        browse
    }

    pub fn super_count(&self) -> usize {
        self.listing.len()
    }

    /// The current super category's sub categories.
    pub fn sub_categories(&self) -> &[(String, Vec<String>)] {
        self.listing.get(self.super_index).map_or(&[], |(_, subs)| subs)
    }

    pub fn sub_count(&self) -> usize {
        self.sub_categories().len()
    }

    /// The current sub category's items.
    pub fn items(&self) -> &[String] {
        self.sub_categories().get(self.sub_index).map_or(&[], |(_, ids)| ids)
    }

    /// The highlighted item (in the item list).
    pub fn item(&self) -> Option<&str> {
        self.items().get(self.item_index).map(String::as_str)
    }

    /// RT / LT (`825D11C0` / `825D1108`): the next or previous super category, wrapping, from its first sub category
    /// and item, on the sub category list. Needs two super categories.
    pub fn step_super(&mut self, forward: bool) -> bool {
        let n = self.super_count();
        if n < 2 {
            return false;
        }
        self.super_index = if forward { (self.super_index + 1) % n } else { (self.super_index + n - 1) % n };
        self.sub_index = 0;
        self.item_index = 0;
        self.remembered = vec![0; self.sub_count()];
        self.subs = true;
        true
    }

    /// Up / Down on the sub category list (`825D1280` / `825D1310`): the previous or next sub category, wrapping; its
    /// remembered item comes back.
    pub fn step_sub(&mut self, forward: bool) -> bool {
        let n = self.sub_count();
        if n == 0 {
            return false;
        }
        self.sub_index = if forward { (self.sub_index + 1) % n } else { (self.sub_index + n - 1) % n };
        self.item_index = self.remembered.get(self.sub_index).copied().unwrap_or(0).min(self.items().len().saturating_sub(1));
        true
    }

    /// Up / Down on the item list (`825D13A8` / `825D1410`): the previous or next item, wrapping, remembered for the
    /// sub category. Down needs two items; Up moves on one as well (it stays).
    pub fn step_item(&mut self, forward: bool) -> bool {
        let n = self.items().len();
        if n == 0 || (forward && n < 2) {
            return false;
        }
        self.item_index = if forward { (self.item_index + 1) % n } else { (self.item_index + n - 1) % n };
        if let Some(r) = self.remembered.get_mut(self.sub_index) {
            *r = self.item_index;
        }
        true
    }

    /// Changes the highlighted item's id (the Size key swaps it for its size variant).
    pub fn replace_item(&mut self, id: &str) {
        let (s, i, j) = (self.super_index, self.sub_index, self.item_index);
        if let Some(slot) = self.listing.get_mut(s).and_then(|(_, subs)| subs.get_mut(i)).and_then(|(_, ids)| ids.get_mut(j)) {
            *slot = id.to_owned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing() -> Listing {
        let sub = |name: &str, n: usize| (name.to_owned(), (0..n).map(|i| format!("{name}{i}")).collect());
        vec![("terrain".into(), vec![sub("street", 3), sub("vert", 1)]), ("props".into(), vec![sub("ramps", 2)])]
    }

    #[test]
    fn keys_walk_the_tree_as_retail() {
        let mut b = Browse::new(listing());
        assert!(b.subs && b.item() == Some("street0"));
        // Sub list: Up wraps to the last sub category.
        assert!(b.step_sub(false));
        assert_eq!(b.sub_index, 1);
        b.step_sub(true);
        // Item list: Down twice, the index is remembered for the sub category.
        b.subs = false;
        b.step_item(true);
        b.step_item(true);
        assert_eq!(b.item(), Some("street2"));
        b.step_item(true);
        assert_eq!(b.item(), Some("street0"), "wraps");
        b.step_item(false);
        b.subs = true;
        b.step_sub(true);
        assert_eq!(b.item(), Some("vert0"));
        // One item: Down does not move, Up stays on it.
        b.subs = false;
        assert!(!b.step_item(true));
        assert!(b.step_item(false));
        b.subs = true;
        b.step_sub(true);
        assert_eq!(b.item(), Some("street2"), "the remembered item");
        // RT: the next super category, from its start, on the sub list; it wraps.
        assert!(b.step_super(true));
        assert_eq!((b.super_index, b.sub_index, b.item_index, b.subs), (1, 0, 0, true));
        b.step_super(true);
        assert_eq!(b.super_index, 0);
        assert_eq!(Browse::new(vec![listing().remove(0)]).step_super(true), false, "one super category");
    }
}
