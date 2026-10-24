//! The loading screen's lines: the `tips_loader/default` record (sub_82605800; research b581). Details:
//! docs/hails-additions/31-retail-menus.md#loading-screen-fe-52
use crate::collections::Collections;

const CLASS: &str = "Hash_F5C0C1FDEC3E475E";
const RECORD: &str = "default";
/// Coach Frank's lines (Text array, 0x4D69B97002FE67B1).
const COACH: &str = "Hash_4D69B97002FE67B1";
/// The hints (`Sk8::FE::TipsLoaderHint` array of title / text, 0x3636A2D5841CBCA0).
const HINTS: &str = "Hash_3636A2D5841CBCA0";

/// String ids, in the record's order (the pick indexes these). skate_vita: serialised as `private/tips-loader.json` in
/// ui.pak (`vita-bake ui`), as sim.pak's collections do not carry the hints.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TipsLines {
    pub coach: Vec<String>,
    pub hints: Vec<(String, String)>,
}

/// The record's lines; an install from before the hint decode has no hints.
pub fn read(collections: &Collections) -> TipsLines {
    let array = |name| collections.field(CLASS, RECORD, name).ok().and_then(|f| f.array.as_ref());
    TipsLines {
        coach: array(COACH).map(|a| a.text_items.clone()).unwrap_or_default(),
        hints: array(HINTS).map(|a| a.text_pairs.iter().map(|[t, x]| (t.clone(), x.clone())).collect()).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Data-gated: an install converted with the hint decode.
    #[test]
    #[ignore = "needs the private install data"]
    fn the_retail_tips() {
        let root = std::path::PathBuf::from(std::env::var("SKATE3_ASSET_ROOT").expect("SKATE3_ASSET_ROOT"));
        let lines = read(&Collections::load(&root).unwrap());
        assert_eq!(lines.coach.len(), 20);
        assert_eq!(lines.coach[4], "ID_TIPS_COACHFRANK_20", "the record's order");
        if lines.hints.is_empty() {
            eprintln!("install from before the hint decode: no hints");
            return;
        }
        assert_eq!(lines.hints.len(), 31);
        assert_eq!(lines.hints[30], ("ID_HINTS_TITLE_31".into(), "ID_HINTS_TEXT_31".into()));
    }
}
