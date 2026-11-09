# Character customization: survey

Nov 2, 2026 · step 1 of the customization phase (survey, then owner decision, then code)

Retail calls it CAC (Create A Character). This survey covers what retail's customization is made of, what upstream
and our code already have, how it ties into the career, and what it would cost on the Vita. Claims are labelled
**[data]** (read from the disc files) or **[code]** (read in the TU3 executable with Ghidra; addresses are TU3
`sub_` names). Nothing from the executable goes in the repo, only addresses, numbers and behaviour in our own words.

## What exists today

| Piece | Where | State |
| --- | --- | --- |
| The skater on the Vita | `skater.pak` (766 KB), `vita-pack/src/skater.rs` | One fixed look: the stock skater from upstream's converted `skater.glb`, textures at most 256 × 256 |
| Edit Skater style values | `skate-sim/src/customiser.rs` (41 lines, from upstream `cb79689`) | Stance, style, posture, truck and wheel settings, and the 4 gesture slots feed the sim from a profile JSON. No screen sets them |
| Free Play / Call Skater (FE 62) | `skate-core/src/spawn_skater.rs`, `skate-ui/src/fe/spawn_skater_host.rs` | Ported; the "You" row is the stock skater |
| Clothing sounds | `skate-audio/src/player/clothing.rs` | Cloth falls, push foley and body slides. They do not depend on the outfit, so customization does not change audio |
| Upstream's customiser | `skate-game/src/customiser*.rs` (about 2,700 lines), `customiser_stamp.wgsl`, `customiser_prepass.wgsl`, `tools/asset_pipeline/customisation_*.py` | Reads the retail catalog, draws the parts, does skin and cloth stamps on the GPU and morphs. **Its menus are upstream's own layout, not retail's screens** (the hierarchy in `customisation_catalog.py` is marked "requested by the user"). Upstream never ported the `createskater` movie. Useful as a reference for decoding the data and for the stamp maths, not for the front end |
| Upstream PR #67 | open | "Use stock skater" takes the customiser outfit off (PC overlay only) |
| The `createskater`, `receipt`, `import_skater`, `team_management` movies | `../game_data/ui-inventory/runtime/*.json` | Already exported by upstream's menu setup. Not in our `ui.pak` and not hosted |

## Retail's data

### The catalog (`createacharacter.big`, 472 MB on the disc) [data]

| File | Size | Contents |
| --- | --- | --- |
| `recipe/createacharacter/cas_db_xfd1ca62f_000.xml` | 7.1 MB | The whole catalog: 3,328 materials and 19 component slots holding 481 part models |
| `cas_db_..._000.unlocks` | 1 KB | 86 unlock names: each is `lookup8(name)` plus a flag word (1 pro board, 16 career path, 17 OTL path, 24 shoe sponsor, 31/63 board-sales group). All 86 decode against the catalog's names |
| `cas_db_..._000.recipe`, `.toc` | 1.8 MB, 68 KB | Binary form of the catalog and the archive index |
| `model/cas_db/<slot>/*.rx2` | 962 files, 32 MB | Part meshes, 2 LODs each |
| `texture/*.rx2` | 3,264 files, 650 MB | Diffuse, normal and stamp textures (mostly 384 KB and 128 KB files, so 512² and 256² with mips) |
| `fedynamic.big` `data/fe/cas/**` | 2,099 files, 36 MB | One shop thumbnail per item (about 17 KB each) |

**Slots and part counts:** OuterTorso 97, Feet 91, Hat 79, Hair 56, Glasses 36, Jewellery 24, Pants 22, WristItem 22,
Leg 18, Accessory 10, Arm 9, Sock 6, SkateBoard 2, Rostral (head) 2, Organ 2, InnerTorso 2, SkateTruck 1,
SkateWheel 1, Misc 1. Each part is tagged male, female or unisex (203 / 188 / 90).

**Materials (3,328):** cloth 806, face stamps 506, cloth "ropa" 476, board graphics 466, cloth stamps 454,
leather 225, skin stamps 138, and smaller groups (hair, alpha, eyes, shift stamps). Most clothing items are a part plus
one of several material variants (`matvar`) per LOD: a T-shirt model with dozens of prints.

**Per-item fields that matter for the career and the shop:**

- `cas.Cost`: price in cash. 2,161 materials have one (most often 30, 50, 45, 40, 60).
- `cas.Unlock`: 936 materials are locked behind one of 23 unlock names (`cac_unlock_bs_group1..22`, board-sales
  groups, plus others). 2,392 have no unlock.
- `cas.SponsorUnlock`: 224 materials, 65 sponsor inventories (`cac_shoe_emerica_inventory`, …).
- `cas.DlcID`, `cas.IsDefault` (the default outfit, 24 materials), `cas.IsUsedInGame`, `cas.ManufacturerID`
  (94 brands, for the brand filter), `cas.StyleSubcategory`, `cas.TattooCategory`.

**Rules between parts** (fields on the part models): `RequiresRemovalOfComponents` (for example a helmet removes Hat
and Hair), `HairModelType` / `RequiresHairModelType` (hat hair vs full hair), `ArmModelRequired` / `LegModelRequired`
(which arm and leg skin pieces show under a top or pants), `RequiresSockStyle`, `InnerCutTypeRequired`,
`IsNecklaceRemoved`, `WearStyle` (22 styles such as "jacket open", "hoody baggy closed hood down sleeves down"),
and stamp UV limits per quadrant. These decide which meshes are drawn together; they must be applied exactly as
retail does, which the code below implements.

### The career link [data]

The progression bank (`db.big` `progressionbanks/main`) holds 222 rewards. About 85 of them are CAC rewards:
`ID_REWARD_CAC_BOARD_*` (28 pro boards), `ID_REWARD_CAC_PATH_*` (19 career paths), `ID_REWARD_CAC_SHOE_*` (14 shoe
lines), `ID_REWARD_CAC_BRANDED_*` (Maloof, Monster, GVR) and `ID_REWARD_CAC_UNLOCK_BS_GROUP1..22`. The string table
gives each one's condition, for example:

- Board-sales groups: "Sell 7,500 boards" (group 1) up to "Sell 1.25 Million boards" (group 22).
- Pro boards: "Beat Koston's pro challenge", "Own the Lot at the Pipe Works".
- Shoe lines: "Beat Cardiel's pro challenge" (Vans), "Complete the Lakai Demo".
- Paths: "Complete 6 Film challenges", "Get the cover of Thrasher".

So the shop's locks are the career's unlock store, and the shop spends the career's cash. Retail's unlock kinds
include 5 CLOTHING_UNLOCK and 6 CLOTHING_SPONSOR (upstream doc 31, from the code; string table at `0x8302CC7C`).

## Retail's screens [code]

| What | Address / id | Notes |
| --- | --- | --- |
| Movie templates | 92 `CreateASkater` (`cas/createskater.swf`), 93 `Receipt` (`cas/receipt.swf`) | Template table at `0x8302F1A8`, 16 bytes a row (same table that gives 107 SpawnSkater) |
| CAC front-end state | object `[[FEmgr+8]+0x3C]`, vtable near `0x823039C8`, code in `825F0000..82606000` (about 158 functions) | Its update `825FAAF0` opens template 92; `82604CB0` opens 93 (the cart) |
| Native registrar | `825A3070` (the `Game` object) | 23 `CAC_*` natives (`825A70B0..825A7BF0`) and 6 `Receipt_*` natives (`825A7C20..825A7EA0`), next to `ColourPicker_*`, `SwatchRenderer_*`, `ImageRenderer_*`, `GetPlayerCash` |
| Menu table | `0x8302FA10`, 48 bytes a row, 37 rows | Each row: internal name, title string id, info-panel type, three layout words, current index, window start, item list. Item lists are filled at run time |

**The 37 menus, in retail order:** Shops, Slappys, Accessories, Skateboards, TattooUpperRegions, TattooLowerRegions,
TattooCategories, TattooUpperAdjust, TattooLowerAdjust, Piercings, PartThumbnails, TattooThumbnails,
GraphicCategories, BrandGraphicThumbnails, TeamGraphicThumbnails, EditSkaterMain, EditSkaterStyle, BackAlleyPete,
Skin, BodyShape, Hair, FacialHair, FaceMod, Eyes, Brow, Nose, Jaw, Mouth, Chin, Presets, WearStyles, Filters,
BrandFilters, ShirtGraphicLocation, ShirtGraphicAdjust, BoardGraphicAdjust, HatGraphicAdjust.

The two shops are **Slappy's** (Merchandise: clothing, boards, accessories, bought with cash through a cart and the
Receipt screen, "Not enough cash!", "You are backing out of Slappy's Merch with items in your cart…") and **Back Alley
Pete's** (Body Mods: skin, body shape, hair, face, tattoos, piercings, "You may only have two piercings"). Edit
Skater's style page holds the stance, the four styles (Normal, Loose, Gonzo, Aggressive), four postures, trucks
(Tight, Medium, Loose), wheels (Hard, Medium, Soft) and the 37 gestures, all of which our sim already reads.

The movies also use the controls `highlight_color`, `highlight_color_grid`, `highlight_morph`, `highlight_slider`,
`swatch_renderer`, `image_renderer` and `gesture_item`.

**Not found yet:** where retail opens the CAC state. No crossbar item does (the 47 crossbar items in
`menu-tables.json` have none). The likely entries are the career team screen (`team_management`, "Customize
Skater") and the career start; this is the first thing to trace in step 3 below.

## How retail builds the look (what the Vita has to do)

1. **Pick the parts** from the outfit record, apply the rules above, and choose the LOD.
2. **Stamp the textures:** skin stamps (tattoos, skin tone, hairline shadow), face stamps (eyebrows, facial hair) and
   cloth stamps (logos and graphics on shirts, hats and boards) are drawn into the part's texture. Upstream does
   this on the GPU (`customiser_stamp.wgsl`).
3. **Morph the face** (FaceMod pages: eyes, brow, nose, jaw, mouth, chin; facial presets) and pick the body-shape
   material variant (thin, average, chubby, thick muscle, …).

On the Vita, steps 2 and 3 only need to run when the outfit changes: stamp once into a render target (vitaGL
framebuffer), and blend the morph on the CPU once into a static mesh. Nothing extra runs per frame while skating.

## Cost on the Vita

| Item | Estimate | Notes |
| --- | --- | --- |
| Card space | Meshes about 20 MB; textures about 200 MB at 256² or about 50 MB at 128²; thumbnails 36 MB (or about 10 MB re-encoded) | A new `cas.pak` read per access from the card, like `ui.pak`. Retail's 650 MB of textures cannot ship as is |
| Memory while skating | About 2 to 4 MB for one outfit (about 10 parts, their textures and the stamped results) | Today's heap: 182 to 223 MB in use in the districts, peak 265 MB of 304 MB. Fits, but must be measured |
| Memory in the CAC screen | The movie (1 to 3 MB like the other screens), about a dozen visible thumbnails, the preview skater | Fine; the world can stay loaded behind it as retail does |
| Draw calls | One per worn part material, about 10 to 15 per skater (today's skater is one mesh) | Small next to the world's 900 to 1,000 |
| CPU while skating | Nothing new | Stamps and morphs only run on change |
| Simulation and parity | None | The style values already feed the sim; changing the look does not touch physics |
| NPC skaters | Unchanged | They keep drawing the stock skater; pros' own models are a separate job |

## What we would reverse-engineer ourselves

- The CAC state's flow: menu tree, item lists per menu, filters, colour pickers, what each native returns, the
  input handling, the cart and receipt, cash checks, the "new item" flag, and the preview camera.
- The outfit record: what retail saves per skater, in which order, and its defaults. Our save format then mirrors it
  (our own file next to `career-stats.json`, as the career survey decided).
- The part rules, applied as the code applies them (not inferred from the field names).
- The stamp layout (where each stamp goes, its UV limits) and the morph data. Upstream's decoding is a good starting
  reference here, checked against the code.
- Where the screen is opened from, and the career start's forced creation (if any).

## Steps

0. **Data layer** (see the decisions below): unlock store, cash, skater record, save. Done (0a `874fd31`, 0b `01495b8`).
1. **Bake:** a `vita-bake cas` step writes `cas.pak`: the catalog (parts, materials, rules, costs, unlock names,
   string ids), the meshes skinned to the stock rig, downsized textures, and the thumbnails. Gate: every part and
   material in the catalog is in the pack (481 parts, 3,328 materials).
   Done for the parts: `cas.pak` (112 MB) from upstream's customiser setup output (run on the install), 480 parts and
   every LOD0 material (2,429 and 170 tattoos); the 459 LOD1 materials are left out (the Vita draws LOD0). Still to
   bake: the 270 logo stamps of `logo_object` (with the Graphics menus) and the shop thumbnails (with the screen).
   The textures are resampled from upstream's full-size PNG decodes, not taken from the retail mips.
2. (Note from the device, 3 Nov: a record's worn slots are not its draw list. Retail composes them first, `8253F648`,
   now `skate_core::cas_rules`, checked against all 18 disc recipes' composed blocks.)
   **Look without a screen:** build the skater from an outfit record (rules, stamps, morphs). The retail default
   outfit replaces today's stock look. Gate: it matches retail's default skater; the device frame time in DownTown
   does not change.
3. **The CAC screen:** the `createskater` movie in `ui.pak`, the CAC state and its natives, Edit Skater's style page
   writing the values the sim already reads, Back Alley Pete's body mods, then Slappy's with the cart (Receipt
   screen) and cash. Entry points traced in the code first.
4. **Locks and cash from the career:** the shop asks the unlock store and the cash balance. This needs the unlock
   store from the career foundation (Phase 5, step 3, not built yet).
5. **Save:** the outfit, style values and purchases saved, and loaded for the Free Play "You" row.

## Owner decisions (2 Nov 2026)

Work happens on the branch `cac-customization` (from `main` at `407bb68`, with the career survey brought over).

1. **Built for the career from the start.** The first step is the career's own data layer in `skate-core`, before
   any screen: the unlock store (retail's unlock names and kinds), the cash balance, and the skater record (outfit
   and style values, a list of skaters so the career team and the Free Play "You" row reuse it). The shop only asks
   "is it unlocked" and "how much cash"; the career later grants its rewards into the same store (the disc maps each
   `ID_REWARD_CAC_*` reward to its unlock name), and the Extras cheat grants land there too.
2. **Locks as on a fresh retail profile,** with a debug "unlock all" setting for testing (like the Object Dropper's
   debug list).
3. **Textures that work on the Vita:** base colour only (plus the decal and alpha textures the stamps need; no normal
   or specular maps, which our skater renderer does not use), at most 256², taken from retail's own mip levels where
   the files have them (no re-compression), DXT as today. Stamps drawn once into a 256² texture when the outfit
   changes. Estimated 100 to 170 MB on the card; measured in the bake step.
4. **Order:** the data layer (desktop tests only), the `cas.pak` bake, the look, then the screen: Edit Skater's style
   page first, then Back Alley Pete's body mods, then Slappy's with the cart and cash. Every behaviour checked in
   Ghidra.

## Slappy's (7 Nov 2026)

Research in the TU3 code before any code (addresses are TU3 `sub_` names):

- **No cash.** Picking a thumbnail wears the item with no price check (825FD9F0 → 825375C0); the parts' info panel
  types are only owned, locked, dlc and none (825FF5E8); `CAC_GetReceiptBalance` returns an empty text (825A7BF0);
  the "not enough cash" and "revert Slappy's" strings are never used by the code. The Receipt screen (FE 16, template 93,
  `Receipt_*` natives) and the cash fields are left over and not reached. The owner checked retail: no cash; career
  clothes show a lock icon until a challenge unlocks them, DLC clothes show their own icon. The cart and receipt of
  step 3 above are therefore dropped.
- **Lists** (82603480): each category row's class object takes one slot (shirts also by top type, socks without
  "tiny"), then the gender, the hidden packs, the hidden-when-locked unlocks, the installed DLC and "not the worn-in
  copy"; one row per style variant (82539188) and retail's brand order (826035F0). The worn item comes first, with the
  "remove" row before it on the slots that can be empty.
- **Trying on:** 500 ms on a thumbnail shows the item on the skater without changing the record (825FEDB8, 825FBAE8,
  825FEEA0); leaving the page shows the record again (826008C0).
- **Exit:** Back on Edit Skater asks `ID_CAC_EXIT_CAC_CONFIRMATION` first (825FC768, 82603290, answer 82604748).

**Owner decisions (7 Nov):**

1. No cart, no receipt, no cash: items are gated by retail's locks (and the debug unlock-all) only.
2. Slice 3a: Slappy's menus, cameras, the exact lists, trying on, wearing, locks, new markers, the unlock text, the exit
   confirmation and the thumbnails (exported from `fedynamic.big`, loaded per page). Slice 3b: Wear Styles (menu 30),
   the filters (menus 31 and 32) and the brand logos in the info panel. Later: the graphics and logos, the glasses
   blend, the boards' graphics.
3. Trying on rebuilds the look on the main thread as Back Alley Pete's does; measured on the device, moved to a thread
   only if the hitch shows.
