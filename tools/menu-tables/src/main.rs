//! Writes the retail front-end menu tables (`menu-tables.json`: the pause menu's categories and
//! items, the Game Settings rows and screens, the Apt key table, the per-mode menus and the Trick
//! Guide's tables) from your own `default.xex`, as skate-3-rust-engine's setup does
//! (`skate3rust --extract-menu-tables`). The executable is unpacked and read by the reference
//! checkout's `skate_data::menu_tables`, built here from that checkout: this repo holds no part of
//! it.
//!
//! usage: menu-tables <default.xex> <menu-tables.json>
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [xex, out] = &args[..] else {
        eprintln!("usage: menu-tables <default.xex> <menu-tables.json>");
        std::process::exit(2);
    };
    let run = || -> Result<String, String> {
        let bytes = std::fs::read(xex).map_err(|e| format!("{}: {e}", xex.to_string_lossy()))?;
        let json = skate_data::menu_tables::json_from_xex(&bytes)?;
        let tables = skate_data::menu_tables::MenuTables::from_json(&json)?;
        if let Some(parent) = std::path::Path::new(out).parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        std::fs::write(out, &json).map_err(|e| format!("{}: {e}", out.to_string_lossy()))?;
        Ok(format!(
            "{}: {} pause items, {} settings rows, {} modes",
            out.to_string_lossy(),
            tables.items.len(),
            tables.settings_rows.len(),
            tables.modes.len()
        ))
    };
    match run() {
        Ok(summary) => println!("{summary}"),
        Err(error) => {
            eprintln!("menu-tables: {error}");
            std::process::exit(1);
        }
    }
}
