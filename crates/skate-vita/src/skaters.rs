//! The created skaters: `ux0:data/SKATEVITA/skaters.json`, our own format for retail's `CACBackEnd::Data` (5 character
//! records, `skate_core::skater_record`). A slot is in use when its head (Rostral) is worn; retail also asks for its
//! composed character block (+0x2000), which we rebuild from the record and the catalog instead of saving it. The main
//! thread writes the file when it changed and on quit.
use serde_json::{json, Value};
use skate_core::skater_record::{ColourLayer, Gender, Graphic, GraphicAdjust, Item, SkaterRecord, Slot, COLOUR_LAYERS, SLOTS};

pub const PATH: &str = "ux0:data/SKATEVITA/skaters.json";
const VERSION: u64 = 1;
/// Records kept (retail: 5 at 0x8309A800).
pub const RECORDS: usize = 5;

pub type Skaters = [Option<SkaterRecord>; RECORDS];

/// Retail's in-use test (82519BD0) without the composed block: a head is worn. For the Free Play "You" row and the
/// career team (not wired yet).
#[cfg_attr(not(test), allow(dead_code))]
pub fn in_use(r: &SkaterRecord) -> bool {
    !r.worn[Slot::Rostral as usize].is_none()
}

/// A record's Edit Skater values in the profile `skate_sim` reads (`customiser::apply_preferences`).
pub fn sim_profile(r: &SkaterRecord) -> Value {
    let gestures: serde_json::Map<String, Value> = r.gestures.iter().enumerate().map(|(i, g)| (i.to_string(), json!(g))).collect();
    json!({ "truck": r.trucks, "wheel": r.wheels, "stance": r.stance, "style": r.style, "posture": r.posture, "gestures": gestures })
}

fn item(i: &Item) -> Value {
    json!([format!("{:016x}", i.model), format!("{:016x}", i.material)])
}

fn hex(v: &Value) -> Option<u64> {
    u64::from_str_radix(v.as_str()?, 16).ok()
}

fn read_item(v: &Value) -> Option<Item> {
    Some(Item { model: hex(&v[0])?, material: hex(&v[1])? })
}

fn floats<const N: usize>(v: &Value) -> Option<[f32; N]> {
    let a = v.as_array()?;
    (a.len() == N).then(|| std::array::from_fn(|i| a[i].as_f64().unwrap_or(0.0) as f32))
}

fn hexes<const N: usize>(v: &Value) -> Option<[u64; N]> {
    let a = v.as_array()?;
    if a.len() != N {
        return None;
    }
    let mut out = [0u64; N];
    for (o, x) in out.iter_mut().zip(a) {
        *o = hex(x)?;
    }
    Some(out)
}

fn record(r: &SkaterRecord) -> Value {
    let hexs = |v: &[u64]| v.iter().map(|x| format!("{x:016x}")).collect::<Vec<_>>();
    json!({
        "gender": r.gender as u8,
        "body_shape": r.body_shape,
        "trucks": r.trucks,
        "wheels": r.wheels,
        "stance": r.stance,
        "style": r.style,
        "posture": r.posture,
        "gestures": r.gestures,
        "unknown": r.unknown,
        "worn": r.worn.iter().map(item).collect::<Vec<_>>(),
        "closet": r.closet.iter().map(|c| c.iter().map(item).collect::<Vec<_>>()).collect::<Vec<_>>(),
        "colour_keys": hexs(&r.colour_keys),
        "colours": r.colours.iter().map(|c| json!({
            "index": c.index, "primary": c.primary, "secondary": c.secondary, "part": item(&c.part),
        })).collect::<Vec<_>>(),
        "graphics": r.graphics.iter().map(|g| json!({
            "name": g.name, "values": hexs(&g.values), "flags": g.flags,
        })).collect::<Vec<_>>(),
        "adjust": { "words": r.adjust.words, "values": r.adjust.values, "flag": r.adjust.flag },
        "morphs": r.morphs,
    })
}

fn read_record(v: &Value) -> Option<SkaterRecord> {
    let mut r = SkaterRecord::cleared();
    r.gender = if v["gender"].as_u64()? == 0 { Gender::Female } else { Gender::Male };
    r.body_shape = v["body_shape"].as_str()?.to_owned();
    r.trucks = v["trucks"].as_f64()? as f32;
    r.wheels = v["wheels"].as_f64()? as f32;
    let u = |k: &str| v[k].as_u64().and_then(|n| u32::try_from(n).ok());
    (r.stance, r.style, r.posture) = (u("stance")?, u("style")?, u("posture")?);
    let words = |k: &str, out: &mut [u32]| -> Option<()> {
        let a = v[k].as_array()?;
        (a.len() == out.len()).then_some(())?;
        for (o, x) in out.iter_mut().zip(a) {
            *o = u32::try_from(x.as_u64()?).ok()?;
        }
        Some(())
    };
    words("gestures", &mut r.gestures)?;
    words("unknown", &mut r.unknown)?;
    let worn = v["worn"].as_array()?;
    let closet = v["closet"].as_array()?;
    if worn.len() != SLOTS || closet.len() != SLOTS {
        return None;
    }
    for s in 0..SLOTS {
        r.worn[s] = read_item(&worn[s])?;
        r.closet[s] = closet[s].as_array()?.iter().map(read_item).collect::<Option<_>>()?;
    }
    r.colour_keys = hexes::<COLOUR_LAYERS>(&v["colour_keys"])?;
    r.colours = v["colours"]
        .as_array()?
        .iter()
        .map(|c| {
            Some(ColourLayer {
                index: u8::try_from(c["index"].as_u64()?).ok()?,
                primary: floats(&c["primary"])?,
                secondary: floats(&c["secondary"])?,
                part: read_item(&c["part"])?,
            })
        })
        .collect::<Option<_>>()?;
    r.graphics = v["graphics"]
        .as_array()?
        .iter()
        .map(|g| {
            let f = g["flags"].as_array()?;
            Some(Graphic {
                name: g["name"].as_str()?.to_owned(),
                values: hexes(&g["values"])?,
                flags: [u8::try_from(f.first()?.as_u64()?).ok()?, u8::try_from(f.get(1)?.as_u64()?).ok()?],
            })
        })
        .collect::<Option<_>>()?;
    let adjust = &v["adjust"];
    let mut a = GraphicAdjust { flag: u8::try_from(adjust["flag"].as_u64()?).ok()?, ..Default::default() };
    let aw = adjust["words"].as_array()?;
    let av = adjust["values"].as_array()?;
    if aw.len() != 5 || av.len() != 5 {
        return None;
    }
    for i in 0..5 {
        a.words[i] = u32::try_from(aw[i].as_u64()?).ok()?;
        a.values[i] = floats(&av[i])?;
    }
    r.adjust = a;
    r.morphs = v["morphs"].as_array()?.iter().map(|m| m.as_f64().map(|f| f as f32)).collect::<Option<_>>()?;
    Some(r)
}

pub fn encode(skaters: &Skaters) -> Vec<u8> {
    let list: Vec<Value> = skaters.iter().map(|s| s.as_ref().map_or(Value::Null, record)).collect();
    serde_json::to_vec_pretty(&json!({ "version": VERSION, "skaters": list })).unwrap_or_default()
}

/// The saved records; a damaged record is left empty, a damaged or newer file gives none.
pub fn decode(text: &str) -> Skaters {
    let mut out: Skaters = Default::default();
    let Ok(doc) = serde_json::from_str::<Value>(text) else { return out };
    if doc["version"].as_u64() != Some(VERSION) {
        return out;
    }
    for (o, v) in out.iter_mut().zip(doc["skaters"].as_array().into_iter().flatten()) {
        *o = read_record(v);
    }
    out
}

pub fn load() -> Skaters {
    std::fs::read_to_string(PATH).map(|t| decode(&t)).unwrap_or_default()
}

pub fn save(skaters: &Skaters) {
    let _ = std::fs::create_dir_all("ux0:data/SKATEVITA");
    if let Err(e) = std::fs::write(PATH, encode(skaters)) {
        println!("skaters not saved: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_survive_a_save_and_load() {
        let mut r = SkaterRecord::cleared();
        r.gender = Gender::Female;
        r.body_shape = "average".into();
        (r.trucks, r.wheels, r.stance, r.style, r.posture) = (0.7, 0.4, 1, 2, 3);
        r.gestures = [5, 9, 12, 36];
        let head = Item { model: 0xadd5_a352_7085_978f, material: 0x0000_53cd_03e3_8817 };
        r.wear(Slot::Rostral, head);
        r.wear(Slot::Hat, Item { model: 1, material: u64::MAX });
        r.wear(Slot::Hat, Item::NONE);
        r.colour_keys[2] = 0x6d93_4d0e_38c9_5e19;
        r.colours.push(ColourLayer { index: 2, primary: [0.5, 0.25, 1.0], secondary: [0.0; 3], part: head });
        r.graphics.push(Graphic { name: "logo_object_logo_alienworkshop_bw_mat".into(), values: [1, 2, 3], flags: [1, 0] });
        r.adjust.values[4] = [0.1, 0.2, 0.3, 0.4];
        r.morphs[18] = 0.75;
        let skaters: Skaters = [Some(r), None, None, None, None];
        let again = decode(std::str::from_utf8(&encode(&skaters)).unwrap());
        assert_eq!(again, skaters);
        assert!(in_use(again[0].as_ref().unwrap()));
    }

    #[test]
    fn the_sim_reads_the_records_style_values() {
        let mut r = SkaterRecord::cleared();
        (r.trucks, r.wheels, r.stance, r.style, r.posture, r.gestures) = (0.5, 0.25, 1, 2, 3, [9, 8, 7, 6]);
        let p = sim_profile(&r);
        assert_eq!((p["truck"].as_f64(), p["wheel"].as_f64()), (Some(0.5), Some(0.25)));
        assert_eq!((p["stance"].as_u64(), p["style"].as_u64(), p["posture"].as_u64()), (Some(1), Some(2), Some(3)));
        assert_eq!((p["gestures"]["0"].as_u64(), p["gestures"]["3"].as_u64()), (Some(9), Some(6)));
    }

    #[test]
    fn a_damaged_file_has_no_skaters() {
        assert_eq!(decode("garbage"), Skaters::default());
        assert_eq!(decode(r#"{"version":1,"skaters":[{"gender":1}]}"#), Skaters::default());
    }
}
