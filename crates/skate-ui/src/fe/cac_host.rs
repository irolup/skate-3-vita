//! FE 15's natives (the `Game` object's `CAC_*`, registrar 825A3070) while it is on top; docs/customization-survey.md.
//! The screen's logic is `skate_core::cac`: this answers the movie and takes its selection.
use crate::fe::apt_vm::{to_integer, Value, Vm};
use skate_core::cac::Cac;
use skate_core::skater_record::SkaterRecord;

#[derive(Clone, Default)]
pub struct CacHost {
    pub screen: Option<Cac>,
    pub shown: bool,
    /// The cash shown (`GetPlayerCash`).
    pub cash: u32,
    /// What the player leaves the screen with.
    pub finished: Option<SkaterRecord>,
}

impl CacHost {
    /// `language`: the string table, which names a thumbnail (the first of its part names it holds, 82605528).
    pub fn call(&mut self, vm: &mut Vm, language: &std::collections::BTreeMap<String, String>, object: &str, method: &str, args: &[Value]) -> Option<Result<Value, String>> {
        if object != "Game" {
            return None;
        }
        let c = self.screen.as_mut()?;
        let row = |i: usize| to_integer(args.get(i).unwrap_or(&Value::Undefined));
        let text = |t: &str| Value::Text(t.to_owned());
        Some(Ok(match method {
            "CAC_GetTitleText" => text(c.title()),
            "CAC_GetFilterText" => text("#"),
            "CAC_GetCurrentMenuIndex" => Value::Number(f64::from(c.cursor().0)),
            "CAC_GetCurrentMenuMinWindowIndex" => Value::Number(f64::from(c.cursor().1)),
            "CAC_UpdateMenuIndices" => {
                c.set_cursor(row(0), row(1));
                Value::Undefined
            }
            "CAC_GetNumItems" => Value::Number(c.row_count() as f64),
            "CAC_GetItemText" => text(c.row_label(row(0))),
            "CAC_GetDescriptionText" => text(&c.row_description(row(0), &|id| language.contains_key(id))),
            "CAC_GetOptionType" => text(c.row_kind(row(0))),
            "CAC_GetIntegerValue" => Value::Number(f64::from(c.row_integer(row(0)))),
            "CAC_GetStringValue" => text(&c.row_string(row(0))),
            "CAC_GetColours" => {
                let swatches = c.row_colours(row(0)).into_iter().map(|v| Value::Number(f64::from(v))).collect();
                return Some(vm.array(swatches).map(Value::Object));
            }
            "CAC_GetInfoPanelType" => text(c.info_panel(row(0))),
            "CAC_GetBannerType" => text(c.banner()),
            "CAC_GetButtonHelpTextA" => text(c.help_a()),
            "CAC_GetButtonHelpTextB" => text(c.help_b()),
            "CAC_GetItemUnlockHALID" => text(&c.unlock_text(row(0))),
            "CAC_GetThumbnailFileName" => text(&c.thumbnail(row(0))),
            // NOT RETAIL YET (slice 3b): the brand's logo from the vault (825FFA38 > 82604070); "no_image" is retail's
            // answer for a brand without one.
            "CAC_GetSponsorLogoFileName" => text("no_image"),
            "CAC_IsPartNew" => Value::Bool(c.is_new(row(0))),
            // 825A7BF0: TU3's shop has no cash, the wallet reads empty.
            "CAC_GetReceiptBalance" => text(""),
            "CAC_OnThumbnailSelect" => {
                c.select_thumb(row(0));
                Value::Undefined
            }
            "GetPlayerCash" => Value::Number(f64::from(self.cash)),
            "OnMenuSelect" => {
                c.select(row(1));
                Value::Undefined
            }
            // 825FEDB8: the thumbnails' highlight starts the try-on wait (Edit Style's gesture preview is not ported).
            "OnMenuHilight" => {
                c.highlight(row(1));
                Value::Undefined
            }
            "OnMenuGetItemEnabled" => Value::Bool(true),
            _ => return None,
        }))
    }
}
