//! SkateAptString82CA14D0 and cFont828076D0/82808708 layout.
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;

#[derive(Clone, Deserialize)]
pub struct Glyph {
    pub glyph_index: usize,
    pub width: f32,
    pub height: f32,
    pub x_offset: f32,
    pub y_offset: f32,
    pub x_advance: f32,
    pub atlas_bounds: [f32; 4],
}
#[derive(Clone)]
pub struct Font {
    /// Secondary native font drawn over this one (825D6B68 attaches it, 82CA1FD8 draws both).
    pub foreground: Option<Box<Font>>,
    pub texture: String,
    pub size: [u32; 2],
    pub scale: [f32; 2],
    pub offset: [f32; 2],
    pub ascent: f32,
    /// Line advance of wrapped text in font units (bmpFont metric LineHeight; vfunc +72 0x82806470 returns
    /// font+360 x scale. NOT RETAIL YET: font+360 is LineHeight or VAdvanceYMax, equal in every retail font).
    pub line_height: f32,
    /// The FontManager row's line spacing (font+364; futuraheavy 0.9). 1.0 in exports made before it was added.
    pub line_spacing: f32,
    /// skate_vita (memory): shared by every copy of the font (each movie, each library link).
    pub glyphs: Arc<BTreeMap<u32, Glyph>>,
}
#[derive(Clone, Default)]
pub struct TextAssets {
    pub fonts: BTreeMap<i32, Font>,
    /// skate_vita (memory): one table for every movie of a set ([`SharedText`]).
    pub language: Arc<BTreeMap<String, String>>,
    /// Every font of the set by APT name (lower case), for fonts a script names (TextFormat.font on a
    /// createTextField field), and the name FontManager falls back to for a name without a row (`fallback_font`
    /// from the export: sub_82809208 takes the first row whose FileName is not "debug").
    pub named: Arc<BTreeMap<String, Font>>,
    pub fallback: Option<String>,
}

/// skate_vita (memory): the text data a movie set shares (`shared.json`: the language table, the fonts, their native
/// layout and pairs), built once for all its movies. Upstream merged these fields into every movie's JSON and each
/// movie built its own copies (the Vita cannot hold 39 of them); [`TextAssets::with_shared`] gives every movie the
/// same `TextAssets` as `TextAssets::load` of the merged JSON, with the tables shared.
pub struct SharedText {
    language: Arc<BTreeMap<String, String>>,
    /// Every font by its APT name (exact case), with its foreground: what a font character of that name gets.
    by_name: BTreeMap<String, Font>,
    /// What `native_font` answers for the other names (the fonts' JSON tree is not kept: 1.6 MB): its error for a
    /// font of the data it could not build, and for any name the data lacks (the same error for all of them).
    failed: BTreeMap<String, String>,
    absent: String,
    named: Arc<BTreeMap<String, Font>>,
    fallback: Option<String>,
}

impl SharedText {
    pub fn from_json(json: &serde_json::Value) -> Result<Self, String> {
        let language = Arc::new(serde_json::from_value(json["language"].clone()).map_err(|e| e.to_string())?);
        let mut fonts_json = serde_json::Map::new();
        for key in ["fonts", "font_mappings", "font_pairs", "fallback_font"] {
            if let Some(v) = json.get(key) {
                fonts_json.insert(key.to_owned(), v.clone());
            }
        }
        let fonts_json = serde_json::Value::Object(fonts_json);
        let mut by_name = BTreeMap::new();
        let mut failed = BTreeMap::new();
        let mut named = BTreeMap::new();
        if let Some(fonts) = fonts_json["fonts"].as_object() {
            for name in fonts.keys() {
                match native_font(&fonts_json, name) {
                    Ok(mut font) => {
                        font.foreground = foreground(&fonts_json, name)?;
                        by_name.insert(name.clone(), font);
                    }
                    Err(e) => {
                        failed.insert(name.clone(), e);
                    }
                }
                if fonts_json["font_mappings"][name].is_null() {
                    continue;
                }
                let mut font = native_font(&fonts_json, name)?;
                font.foreground = foreground(&fonts_json, name)?;
                named.insert(name.to_lowercase(), font);
            }
        }
        // NOT RETAIL YET (old exports without `fallback_font`): the first name in sorted order stands in.
        let fallback = fonts_json["fallback_font"].as_str().map(str::to_lowercase).or_else(|| named.keys().next().cloned());
        let fonts = fonts_json["fonts"].as_object();
        let probe = (0..).map(|i| format!("\u{0}absent{i}")).find(|n| !fonts.is_some_and(|f| f.contains_key(n))).unwrap_or_default();
        let absent = native_font(&fonts_json, &probe).err().unwrap_or_default();
        Ok(Self { language, by_name, failed, absent, named: Arc::new(named), fallback })
    }

    /// The font a font character named `name` gets (`native_font` + `foreground`, as `TextAssets::load`).
    fn font(&self, name: &str) -> Result<Font, String> {
        match self.by_name.get(name) {
            Some(font) => Ok(font.clone()),
            None => Err(self.failed.get(name).unwrap_or(&self.absent).clone()),
        }
    }

    pub fn language(&self) -> &Arc<BTreeMap<String, String>> {
        &self.language
    }
}

/// Font ids handed to script-named fonts in `TextAssets::fonts` (below every character id).
pub const NAMED_FONT_BASE: i32 = -1_000_000;
/// Retail's one secondary-font rule, 825D6B68: an APT font named "Futura Shadow" (8220BD44, stricmp
/// 82AE89B0) gets the native font "futuraheavy" (8220BD54) from the global font table (82809208), whether
/// or not the movie itself uses that font. Data can replace it (`font_pairs`: APT name -> native file name).
pub const RETAIL_FONT_PAIRS: &[(&str, &str)] = &[("Futura Shadow", "futuraheavy")];

/// The native file a font named `apt_name` pairs with (`font_pairs` in the data, else retail's table).
pub fn paired_file(json: &serde_json::Value, apt_name: &str) -> Option<String> {
    match json.get("font_pairs").and_then(|p| p.as_object()) {
        Some(pairs) => pairs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(apt_name))
            .and_then(|(_, v)| v.as_str())
            .map(str::to_owned),
        None => RETAIL_FONT_PAIRS
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(apt_name))
            .map(|(_, v)| (*v).to_owned()),
    }
}

/// One font from the movie's font data, by its APT name (FontManager 82808AE8 row + bitmap bank).
fn native_font(json: &serde_json::Value, name: &str) -> Result<Font, String> {
    let asset = &json["fonts"][name];
    let d = &asset["definition"];
    let glyphs: Vec<Glyph> =
        serde_json::from_value(d["glyphs"].clone()).map_err(|e| e.to_string())?;
    let layout = &json["font_mappings"][name]["native_layout"];
    let f = |key: &str| {
        layout[key]
            .as_f64()
            .map(|v| v as f32)
            .ok_or_else(|| format!("Missing native font {name}.{key}"))
    };
    let mut glyph_map = BTreeMap::new();
    let mut font = Font {
        foreground: None,
        texture: asset["texture"]
            .as_str()
            .ok_or("Font texture missing")?
            .into(),
        size: [
            d["textures"][0]["width"]
                .as_u64()
                .ok_or("Font width missing")? as u32,
            d["textures"][0]["height"]
                .as_u64()
                .ok_or("Font height missing")? as u32,
        ],
        scale: [f("ScaleX")?, f("ScaleY")?],
        offset: [f("OffsetX")?, f("OffsetY")?],
        ascent: d["metrics"]["Ascent"]
            .as_f64()
            .ok_or("Font ascent missing")? as f32,
        line_height: d["metrics"]["LineHeight"].as_f64().unwrap_or(0.0) as f32,
        line_spacing: layout["LineSpacing"].as_f64().unwrap_or(1.0) as f32,
        glyphs: Arc::default(),
    };
    for mapping in d["characters"]
        .as_array()
        .ok_or("Font character map missing")?
    {
        let index = mapping["glyph_index"]
            .as_u64()
            .ok_or("Invalid font glyph index")? as usize;
        let glyph = glyphs
            .iter()
            .find(|g| g.glyph_index == index)
            .ok_or("Absent mapped glyph")?;
        glyph_map.insert(
            mapping["codepoint"]
                .as_u64()
                .ok_or("Invalid font codepoint")? as u32,
            glyph.clone(),
        );
    }
    font.glyphs = Arc::new(glyph_map);
    Ok(font)
}

/// The secondary font for `apt_name`, looked up in the whole native font table by file name.
/// NOT RETAIL: when the paired font is absent (a mod's data, an old export) retail's lookup falls back to
/// the first row whose FileName is not "debug" (82809208 loop 2; its compare 82AE8A10 returns 0 on a match);
/// here the text draws its primary pass only. (The menu export applies that fallback to APT names without a
/// row, e.g. the trick guide's HelveticaPresto: prepare_menu_movies.py.)
fn foreground(json: &serde_json::Value, apt_name: &str) -> Result<Option<Box<Font>>, String> {
    let Some(file) = paired_file(json, apt_name) else {
        return Ok(None);
    };
    let Some(name) = json["font_mappings"].as_object().and_then(|m| {
        m.iter()
            .find(|(_, row)| {
                row["file_name"]
                    .as_str()
                    .is_some_and(|f| f.eq_ignore_ascii_case(&file))
            })
            .map(|(name, _)| name.clone())
    }) else {
        return Ok(None);
    };
    if json["fonts"][&name].is_null() {
        return Ok(None);
    }
    Ok(Some(Box::new(native_font(json, &name)?)))
}

impl TextAssets {
    /// skate_vita: upstream's `load` is [`Self::with_shared`] over the movie's own shared fields.
    pub fn load(json: &serde_json::Value) -> Result<Self, String> {
        Self::with_shared(json, &SharedText::from_json(json)?)
    }

    /// skate_vita (memory): the movie's text assets with the set's shared text (upstream `load` of the movie JSON
    /// with `shared` merged in: the same fonts per character, named fonts, fallback and language).
    pub fn with_shared(json: &serde_json::Value, shared: &SharedText) -> Result<Self, String> {
        let mut out = Self { language: Arc::clone(&shared.language), named: Arc::clone(&shared.named), fallback: shared.fallback.clone(), ..Self::default() };
        for c in json["characters"]
            .as_array()
            .ok_or("HUD characters missing")?
        {
            if c["type_name"] != "font" {
                continue;
            }
            let name = c["font"]["name"].as_str().ok_or("HUD font name missing")?;
            out.fonts.insert(
                c["id"].as_i64().ok_or("Invalid font character id")? as i32,
                shared.font(name)?,
            );
        }
        Ok(out)
    }

    /// The font id for a script-named font (case-insensitive, else the FontManager fallback), added to `fonts`
    /// on first use. None when the set has no fonts at all.
    pub fn font_named(&mut self, name: &str) -> Option<i32> {
        let key = name.to_lowercase();
        let key = if self.named.contains_key(&key) { key } else { self.fallback.clone()? };
        let index = self.named.keys().position(|k| *k == key)? as i32;
        let id = NAMED_FONT_BASE - index;
        if !self.fonts.contains_key(&id) {
            self.fonts.insert(id, self.named[&key].clone());
        }
        Some(id)
    }
    pub fn localize(&self, text: &str) -> String {
        if let Some(literal) = text.strip_prefix('#') {
            return literal.into();
        }
        self.language
            .get(text)
            .cloned()
            .unwrap_or_else(|| text.into())
    }
}
impl Font {
    pub fn glyph(&self, c: char) -> Option<&Glyph> {
        self.glyphs
            .get(&(c as u32))
            .or_else(|| self.glyphs.get(&65535))
    }
    pub fn width(&self, text: &str, height: f32) -> f32 {
        text.chars()
            .filter_map(|c| self.glyph(c))
            .fold(0.0, |x, g| g.x_advance.mul_add(self.scale[0] * height, x))
    }
    /// The distance between wrapped lines (82CA2118: vfunc +72 x font+364 x height).
    pub fn line_advance(&self, height: f32) -> f32 {
        self.line_height * self.scale[1] * self.line_spacing * height
    }
    /// Retail's word wrap (SkateAptString 82CA14D0, loop 82CA1BB0): greedy, measuring from the line start through
    /// each character; a space is the only break point (one at index 0 is not), `^` forces a break, and the break
    /// character is dropped. A word wider than the field is never split; `\n` is an ordinary character.
    pub fn wrap(&self, text: &str, height: f32, width: f32) -> Vec<String> {
        let chars: Vec<char> = text.chars().collect();
        let mut lines = Vec::new();
        let (mut start, mut i, mut candidate) = (0, 0, 0);
        while i < chars.len() {
            let forced = chars[i] == '^';
            if forced || chars[i] == ' ' {
                candidate = i;
            }
            let measured: String = chars[start..=i].iter().collect();
            if forced || (self.width(&measured, height) > width && candidate != 0 && candidate >= start) {
                lines.push(chars[start..candidate].iter().collect());
                start = candidate + 1;
                i = start;
                candidate = 0;
                continue;
            }
            i += 1;
        }
        lines.push(chars[start.min(chars.len())..].iter().collect());
        lines
    }
}

#[cfg(test)]
mod font_pair_tests {
    use super::*;
    use serde_json::json;

    fn asset(texture: &str) -> serde_json::Value {
        json!({"texture": texture, "definition": {
            "glyphs": [{"glyph_index": 0, "width": 1.0, "height": 1.0, "x_offset": 0.0, "y_offset": 0.0,
                        "x_advance": 1.0, "atlas_bounds": [0.0, 0.0, 1.0, 1.0]}],
            "characters": [{"glyph_index": 0, "codepoint": 65}],
            "textures": [{"width": 8, "height": 8}], "metrics": {"Ascent": 1.0}}})
    }
    fn row(file: &str) -> serde_json::Value {
        json!({"file_name": file, "native_layout": {"ScaleX": 1.0, "ScaleY": 1.0, "OffsetX": 0.0, "OffsetY": 0.0}})
    }
    /// A movie that places only "Futura Shadow" (small_popup's case), with the native table holding both.
    fn movie() -> serde_json::Value {
        json!({"language": {}, "characters": [{"id": 3, "type_name": "font", "font": {"name": "Futura Shadow"}}],
               "fonts": {"Futura Shadow": asset("shadow.rgba"), "Futura Std Medium": asset("heavy.rgba")},
               "font_mappings": {"Futura Shadow": row("futurashadow"), "Futura Std Medium": row("futuraheavy")}})
    }

    #[test]
    fn shadow_gets_futuraheavy_from_the_native_table_without_a_character() {
        let t = TextAssets::load(&movie()).unwrap();
        assert_eq!(t.fonts.len(), 1);
        assert_eq!(t.fonts[&3].foreground.as_ref().unwrap().texture, "heavy.rgba");
    }

    #[test]
    fn missing_partner_draws_primary_only() {
        let mut m = movie();
        m["fonts"].as_object_mut().unwrap().remove("Futura Std Medium");
        assert!(TextAssets::load(&m).unwrap().fonts[&3].foreground.is_none());
        m["font_mappings"].as_object_mut().unwrap().remove("Futura Std Medium");
        assert!(TextAssets::load(&m).unwrap().fonts[&3].foreground.is_none());
    }

    #[test]
    fn data_pairs_replace_the_retail_table_case_insensitively() {
        let mut m = movie();
        m["font_pairs"] = json!({"futura shadow": "FUTURAHEAVY"});
        assert!(TextAssets::load(&m).unwrap().fonts[&3].foreground.is_some());
        m["font_pairs"] = json!({});
        assert!(TextAssets::load(&m).unwrap().fonts[&3].foreground.is_none());
        m["font_pairs"] = json!({"Futura Shadow": "no_such_font"});
        assert!(TextAssets::load(&m).unwrap().fonts[&3].foreground.is_none());
    }

    #[test]
    fn other_fonts_get_no_partner() {
        let mut m = movie();
        m["characters"][0]["font"]["name"] = json!("Futura Std Medium");
        assert!(TextAssets::load(&m).unwrap().fonts[&3].foreground.is_none());
    }

    /// 82CA1BB0: break at the last space once the line is too wide, `^` forces a break, a long word overhangs.
    #[test]
    fn wrap_follows_the_retail_break_rule() {
        let glyph = Glyph { glyph_index: 0, width: 1., height: 1., x_offset: 0., y_offset: 0., x_advance: 1., atlas_bounds: [0.; 4] };
        let font = Font {
            foreground: None,
            texture: String::new(),
            size: [1, 1],
            scale: [1., 1.],
            offset: [0., 0.],
            ascent: 0.,
            line_height: 50.,
            line_spacing: 0.9,
            glyphs: Arc::new([(65535, glyph)].into_iter().collect()),
        };
        // One unit per character at height 1.
        assert_eq!(font.wrap("aaa bbb ccc", 1., 7.), ["aaa bbb", "ccc"]);
        assert_eq!(font.wrap("aaa^bbb", 1., 50.), ["aaa", "bbb"]);
        assert_eq!(font.wrap("aaaaaaaaaa bb", 1., 4.), ["aaaaaaaaaa", "bb"]);
        assert_eq!(font.wrap("short", 1., 50.), ["short"]);
        assert!((font.line_advance(2.) - 90.).abs() < 1e-4);
    }
}

/// 825E51A0 localizes each authored component before composing a literal.
pub(crate) fn localize_trick(label: &str, assets: Option<&crate::fe::apt_text::TextAssets>) -> String {
    if let Some(literal) = label.strip_prefix('#') {
        return literal.to_owned();
    }
    label
        .split_whitespace()
        .map(|part| {
            let text = assets
                .map(|a| a.localize(part))
                .unwrap_or_else(|| part.to_owned());
            if text.starts_with("ID_") {
                humanize_trick_id(&text)
            } else {
                text
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn humanize_trick_id(id: &str) -> String {
    let rest = id
        .strip_prefix("ID_TRICK_")
        .or_else(|| id.strip_prefix("ID_"))
        .unwrap_or(id);
    rest.split('_')
        .filter(|word| !word.is_empty())
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            let mut chars = lower.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
