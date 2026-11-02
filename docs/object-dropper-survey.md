# Object Dropper survey

28 Oct 2026. Step D1, before Phase 5: what retail's Object Dropper is, what data and code it uses, what we
already have, and what it would cost on the Vita. Sources:

- **Code [code]:** the owner's own TU3 executable (`default.xex` + `default.xexp`) read in Ghidra 12.1.4 with the
  Xenon360 0.7.0 loader, following upstream's `ghidra` skill (PR #65). Everything stays local (`~/tools`); this doc
  holds only addresses, constants and behaviour in our own words. Addresses are the recomp's `sub_` addresses.
- **Data [data]:** the disc: `content/parkassets.big`, `big/fedata.big` (`screens/objectdropper/`), `big/db.big`
  (`skatercollections`), read with upstream's BIG reader; the converted vault (`skater-collections.json`).
- **Upstream:** doc 27 "Object Dropper and reset" on PR #52 (`0cd26b08`); PR #15 (dynamic props, draft, idle since
  29 Sep, already in our tree in a newer form).

Credits: Ghidra (NSA), Xenon360 (EveryoneEverybody), skate3recomp / rexglue / Xenia for the addresses, and upstream
(Hailey-Ross) for doc 27.

## The main findings

1. **One editor, two places: the open world ("Object Dropper") and the skate.Parks ("Edit Park").** The phone
   (`826682B0`) handles the dropper row on FE input `0x40`: when `82668EB8` allows it (game mode not 4 or 5,
   `IsObjectDropperEnabled`, and not busy with the photographer or another player's session), it closes the phone
   and posts flow message `0x7A` in the open world, or, in a skate.Park, checks the park's unlocks (the "not
   unlocked in your career" / DLC popups) and posts `0x79` [code]. The skate-park flag (`82511168`: field
   `0x0C54AD24155E3220` of the `world` record) is set by `default_skateparks` and inherited by StartPark,
   DownTownPark, IndustrialPark and MegaPark; every district has it off [data]. The editor's own code
   (`0x82870000-0x82900000`) never reads that flag, and the catalogue builder (`82880708`) takes every
   `fe_park_categories` record, so the open world gets the same editor and catalogue (minus locked items).
   **Correction to doc 27:** FE input `0x100` with gate `826691D0` is not the dropper but **Reset All Props**
   (`ID_PHONELIST_RESET_ALL_OBJECTS`, getter `GetPhoneListCanResetAllObjectsOption` = `82661180`): it calls the
   DMO manager at `0x830854A8` (vfunc +8, then +0x14) and is offered only in skate.Parks; the row getters are
   registered by `8265B870` (`GetPhoneListCanShowObjectDropperOption` = `82661100` → `82668EB8`).
2. **The catalogue is data, not code:** 661 items in a three-level tree (see "Catalogue").
3. **Retail limits placement with a "Feng Shui" meter:** two bars, **Props** (physics objects) and **Terrain**
   (park pieces), each a fraction of a budget, with a per-item cost (`FPSCost`). The bar turns colour at 0.75 and
   0.9, a warning plays above 0.9, and the screen shows "Warning!" (`ID_SKATEPARK_OBJECTLIMIT_WARNING`) [code][data].
   This is the hook for Vita limits: same meter, Vita-sized budgets.
4. **The controls you described are retail's.** The help strings on the disc give every button (table below).
5. **The editor's tuning is in the vault:** `objectdropper` (one record per mode: default, free_cam, manipulate,
   object_spawner), `editor_cursor` and `editor_camera` records, plus selection and axis colours
   (`skatepark_presentation`) [data]. The values are known; most field meanings still need reading in the code.

## Upstream

- **PR #15** (draft, last push 29 Sep): dynamic props phases 0-4 (live instances, collision, rigid bodies,
  grab and drag on foot, placement mode, per-map layout file). All of it is in our tree through PR #52, newer than
  the PR (grab on RB, retail Move Object). It is not an Object Dropper: no free camera, no catalogue, no spawning.
- **Doc 27 (PR #52):** decoded part of the phone (its FE input 256 is in fact Reset All Props, see finding 1;
  the dropper is input `0x40`), the per-object Reset / Upright / add-to-marker actions, retail's
  auto-reset of objects that fell out of the world, and the dropper's FE vocabulary. Ported: reset to spawn
  (`reset_prop`), reset all moved props (theirs, not retail), runtime prop spawn / remove (`spawn_runtime_prop`).
  Marked NOT RETAIL YET: the whole editor (catalogue, free camera, placement, snap, group select, delete, quick
  menu). Nobody upstream is working on it.

## What retail's editor does

From the movies `objectdropper.apt` (49 KB) and `objectquickmenu.apt` (11 KB), the FE natives the screen
registers (`825B3CE0`, 58 natives) and the Lua `ObjectDropper` natives (table at `0x823237E0`) [data][code]:

- **Modes:** FreeCam (fly the camera; the skater stays in the scene), Catalog (pick an item: super category tabs,
  sub categories, items with icons, "New!" marks, unlock text), Manipulate (move the held object), Group Select,
  Quick Menu, Visual Customization.
- **Object actions:** place, place and duplicate (clone), duplicate, delete, reset (back to its authored pose),
  rotate, move on a locked axis, fine tune (slower, finer moves), snap to another object (basic / advanced),
  group select (add / remove from group, move several), hide / show selection, size and info pages
  (`GetCurrentItemSizeInfo` = the item's x / y / z size).
- **Quick Menu (Back button, "Properties"):** Color, Brand, Style (Downtown / Industrial / University variants of
  a piece, `fe_park_itemtypes`), Options (Snapping, Collision on/off, Invert X / Y axis, Cursor speed 0-10),
  Copy, Cut, Paste, Undo, Redo.
- **The park's spawner:** every park has one `SkatePark_Spawner` piece (where the skater starts);
  `IsSpawnerPresent` / `IsWorldSpawner` guard it, and `object_spawner` has its own cursor tuning.
- **Challenges drive it too:** Lua `SetCatalogFilter` (`82C62CC8`) takes a list of item ids and restricts the
  catalogue (the park tutorials), plus `IsInDropper`, `IsInCatalogMode`, `IsHoldingItemOfModelID`,
  `IsTransformSimilarToSelectedItem`, ... (25 natives) for tutorial objectives. That is Phase 5 work.

### Controls (retail help strings)

| Action | Retail button | Retail text | Vita proposal |
|---|---|---|---|
| Open | LB + B (phone row, FE input `0x40`) | "Object Dropper" / "Edit park" | rear-left touch (or d-pad left) + Circle, as the phone today |
| Move cursor / object | Left stick | | Left stick |
| Camera | Right stick | | Right stick |
| Up / down | D-pad up / down | `DPad_UpDown` (`GodHand_Cursor_ElevateUp/Down`) | D-pad up / down |
| Zoom in / out | D-pad right / left (`input.cfg` `GodHand_Camera_ZoomIn/Out`) | | D-pad right / left |
| Advanced modifier | LT held (`GodHand_AdvancedModifier`) | | L |
| Select / place | A (hold: group select) | "Select object / Hold for group select", "Place" | Cross |
| New object (catalogue) | X | "New object" | Square |
| Delete | Y | "Delete" | Triangle |
| Back | B | "Back" | Circle |
| Duplicate | LB | "Duplicate", "Clone / Place and Clone" | rear-left touch (the d-pad zooms here) |
| Lock axis, snap modifier | RB (+ left stick, + X to snap) | "Hold RB + left stick to move on a locked axis", "Hold RB + X to snap" | rear-right touch (the d-pad zooms here) |
| Fine tune | LT | "Fine Tune" | L |
| Rotate | RT held (+ stick) | "Hold RT + right stick to rotate" | R |
| Reset object | L3 click | "<L3> Reset" | front-touch left half (the L3 zone) |
| Properties / Quick Menu | Back | "<Back> Properties" | Select |

The disc's `data/config/input.cfg` (`miscboot.big`) gives the editor's analog bindings: moves and camera turns are
the stick half-axes cubed, and **d-pad left / right zoom**, so in the editor LB / RB can only come from the rear
touch (a d-pad stand-in would take the zoom). **Note:** in retail, Select (Back) opens Properties, and "Reset
All Props" is a phone row, not a button in the editor.

## Catalogue

`content/parkassets.big` holds two recipe databases (`static_db`, `dynamic_db`: XML item lists with per-item caps)
and the models and textures [data]. The tree comes from the vault class `fe_park_categories` (`0xC9A74C3F7CAD7CEE`,
read by `825D16B8`; built by `82880708`) [code][data]:

| Super category | Sub categories (items) | Unlock caps |
|---|---|---|
| Props (dynamic, physics) | Ramps 16, Rails 22, Bins 22, Barriers 18, Boxes 8, Tables 6, Misc 27 (119) | none, except 2 sponsor items (Dr Pepper, T-Mobile) |
| Terrain (static) | PreMade 36, Street 99, Vert 93, Plaza 74, Mega 15, Rails 97, Hubbas 54 (468) | `skatepark_unlock_startpark` / `_plaza` / `_industrial` / `_stadium` (career) |
| Misc terrain (static) | Branded 6, Downtown 12, Foliage 7, OTS 24, Misc 16, Teleports 8, + the Spawner (74) | mostly career unlocks; OTS pieces unlock per spot (`skatepark_unlock_ots_*`) |

- Item caps: AdvanceSnap, AllowCollision, DescriptionID, DlcID, **FPSCost**, IgnoreFuser, IsColorizable, IsHidden,
  IsLogoable, Unlock (cap names table at `0x82FCB120`) [code]. Only 7 items set FPSCost (balls and pylons 11000,
  one sign 39); everyone else uses a default from code, not found yet.
- Many terrain items come in lrg / md / sm sizes and Downtown / Industrial / University styles.
- Size on the disc: dynamic 2.6 MB models + 18.2 MB textures; static 14.2 MB models + 33.0 MB textures (Xbox
  formats; the recipe reports 38 MB and 73 MB of GPU memory if everything were loaded). Icons: one 17 KB image per
  item in `fedynamic.big` (`images/objects/`).
- The Fullpark layouts of Phase 2 are built from the same items (each instance carries the catalogue template
  id, name, tint and caps), so a park we load today is already an editable set of catalogue items.

## What the catalogue lists (decoded 29 Oct)

- **The world sets the catalogue's mode** (`828A2D28`, at a world load): a skate.Park world (its `world` park flag)
  gives mode 0, any other world mode 1 (catalogue object `[0x830CFD94]+0x110`, `+0x8C`) [code].
- **Building the lists** (`82880708` then `828803D8` per item, both recipes): in mode 1 a dynamic prop is listed only
  when its template is loaded in the world (`0x830854B0`, the world's DMO template map, `82BFDAC0`: the district's own
  DMO banks); the static pieces come from the skate.Park recipe, which only parks load. So the open world lists the
  district's own props: the owner remembers about 82 [code; count from the owner].
- **The banks (counted 29 Oct):** `worlddmo.big` holds four DMO streams; decoded (upstream `prepare_catalog`), the 119
  catalogue props' templates are in Global (66), DownTown (17), Industrial (15) and University (21), each tagged in
  its name (`DMO_glbl_`, `_DT_`, `_IND_` (one `_lnd_`), `_UN_`). A district loads Global and its own bank: 83
  (DownTown), 81 (Industrial), 87 (University) props, the owner's "about 82" [data]. Ours lists Global plus the map's
  bank (`object_dropper::catalog::bank`); retail may also have a neighbour's bank streamed near a border [inferred].
- **Every item** also passes the challenge whitelist (`SetCatalogFilter`, when set), its DLC (`DlcID` must be owned)
  and its `Unlock` key (`8287FEE8`), then the current **style** (`+0xF8`: the item's name suffix, Downtown /
  Industrial / University, through `fe_park_itemtypes`, `8287E980`) and **size** (`+0x100`, `8287EBA8`): one entry per
  piece family, the Quick Menu's Style and Size pick the variant. Empty sub categories and super categories are
  dropped. Items named `Spawner` are the park's spawner, not listed.
- **Ours (owner):** retail's lists by default; a setting ("All objects") lists every item anywhere, the pieces retail
  keeps to custom parks included.

## Limits: the Feng Shui meter

- The dropper object (`[0x830CFD94]+0x124`, constructor `8288B2E8`) holds the meter at `+0x16F0`, filled each
  update by `8288F1B8`: terrain used (`+0x173C`) and to add (`+0x1740`), props used (`+0x1744`) and to add
  (`+0x1748`), each the largest of the memory fraction and its budget's parts, and the warning flag (`+0x174C`,
  either bar's used + added above 0.75) [code].
- The meter (`825119C8`) draws each bar from those fractions and changes colour at **0.75** and **0.9**
  (`0x821814A0`, `0x820997B8`); the FE (`825C84A8`) posts event `0x58A8DCE7FF78CDFF` (probably the warning
  sound) when props pass 0.9 [code]. The park's
  stats event reports the props fraction × 99 (`825113D8`).
- **The rule (decoded 28 Oct, D3 research):** a placement is allowed only when three budgets all pass
  (`8288EE08` for the selection, `8288EF58` for a given list) [code]. A full meter **refuses** the placement.
  | Budget | Built by / checked by | Limits | Use |
  |---|---|---|---|
  | Memory | `8288E430` / `8288E998` | main and GPU memory, in MiB from the `world` row (`957B82DDC172BA97` main, `D66A899E3366601D` GPU: parks 22 / 75, StartPark 25 / 80); a world without them (every district) 30 / 100 MiB (`0x1E00000`, `0x6400000`) | the loaded items' sizes (the recipe's per-asset MainMem / GFXMem table, `82BF3B68`); used + added ≤ 1 for each |
  | Props | `8288EA18` / `8288EB40` | **47** dropped props (`0x2F`), **48** live objects (`0x30`) | max(added / 47, added / 48) + max(dropped / 47, live / 48) ≤ 1; dropped = the DMO manager's count (`+0x1D50`), live = those of its objects that are not asleep or parked |
  | Terrain | `8288EBE8` / `8288ED40` | cost **200,000**, **748** pieces (`0x2EC`) | max(cost / 200,000, pieces / 748) + max(added cost / 200,000, added pieces / 748) ≤ 1; cost per piece `82865828` = its model's mesh command count (`82865778` / `82865698`), summed when a piece is added (`828662B8`: piece manager `+0x3B50` cost, `+0x3B54` count) |
- The recipe's `FPSCost` cap (7 items) is not read by any of these; where it acts is still open.
- **Vita:** keep the three-budget rule and the meter, with our own units: memory from each item's baked render and
  texture bytes (`catalog.json` costs), terrain cost from triangles and draws, props from awake bodies the sim
  tick can afford; the limits measured on the device (D6).

## Tuning in the vault

| Record | Values (fields still unnamed) | Read by |
|---|---|---|
| `objectdropper/default` | 1, 40, 0.15, 62.5, 0, 0.5, 0.03, 0.04, a curve, 0.04, 0.7 | `8288C310`, `8288CE40`, `8288D488` |
| `objectdropper/free_cam` | 0.1, 0.3, on, 0.3, 0.02, 0.04, 0.04, 1 (overrides) | same |
| `objectdropper/manipulate`, `object_spawner` | small overrides | same |
| `editor_cursor/objectdropper` | 9, 7, 0.02, 0.4, 0.7, 0.02, 200, 0.35, 45, 1.5, a colour | `828871B8`, `82887568`, `82887B78`, `828883D8`, `82889F58`, `8288A5E0` |
| `editor_camera/objectdropper` | -80, 80, 60, ±20, 40, 80, 40, 5, 5, 5, 0.7, 1, 0.05 | `828769E0`, `828863A0`, `82886CC8` |

Cursor speed is a float at `+0x17B0`; the Quick Menu shows it as `(value - 0.4999) × 10`, clamped to 0-10
(`825B5730`) [code]. The values can be baked
as they are; each function above tells which value is which (pitch limits, heights, speeds, snap distance, ...).

Movement (`8288C310`, first read): the stick's move is scaled by a curve (`objectdropper` `D6BCD32064145E72`) of
the camera's height between two `editor_camera` heights, times the cursor speed; `BF2D64741246823F` scales the
horizontal move and `23E188CB1C0B3760` the vertical one; a held direction ramps up (`+0x17A8`); bits 0-2 of
`+0x1770` lock the x, y and z axes (Lock Axis). The rest is decoded with each feature in D3.

## What we have already

| Piece | Where | State |
|---|---|---|
| LB phone panel with the dropper row | `skate-vita/src/retail_hud.rs`, `skate-sim/src/session_marker/` | drawn; row not wired |
| Dynamic props: bodies, collision, grab and carry, placement, per-map layout file | `skate-sim/src/physics/prop_dynamics.rs`, `prop_carry.rs`, `prop_layout.rs` | done (L1) |
| Runtime spawn / remove of a prop, reset to spawn, reset all moved | `GamePhysics::spawn_runtime_prop`, `reset_prop`, `reset_moved_props` | done, no UI |
| Park layouts (Fullpark), pieces from the catalogue | `skate-sim/src/park_layout.rs`, `park_bake.rs`, `vita-bake/src/park_layout.rs` | done (Phase 2), **merged meshes and fused collision baked offline** |
| APT VM and retail screens | `skate-ui/src/fe/` | done (R2); the dropper movies are new screens |
| Free camera | replay editor (P1) | done; a model for the editor camera |

## Vita cost and approach

- **Data (bake):** a catalogue pack with the item table (tree, caps, unlocks, sizes) and per-item meshes,
  collision and textures in the Vita formats, read **on demand** (only placed and held items are in memory), plus
  the icons. Card: roughly 25-35 MB after DXT and halved textures. RAM: per item ~50-300 KB; a park with 50
  distinct new items ~5-10 MB. Parks peak at 160 MB today, so there is room, but a cap is needed.
- **Render:** Phase 2 merges the park's pieces per material and 32 m cell. An edited piece has to leave its cell
  mesh: bake per-piece index ranges into the cell meshes so a cell can be drawn without a piece (one more draw),
  and draw moved, added and held pieces as instances. Each extra draw costs 10-75 µs on the Vita (earlier
  measurements); the Stadium is at 238 draws / 11 ms CPU today. This, more than memory, is the budget.
- **Collision:** the Fullpark collision is fused at bake. Plan: tag fused triangles with their piece at bake;
  moving or deleting a piece parks its triangles (as the held prop does today); a placed piece gets its own
  unfused triangles, and a background re-fuse of the area around it removes seams (the fuser takes 89-345 ms for a
  whole park on the desktop, so a local area is quick).
- **Open world:** districts have no fused park to edit, so everything dropped there is an instance with its own
  unfused collision, added to the streamed district's collision window and drawn after the tiles. Districts are
  already the tightest place on the Vita (DownTown sim near 16.7 ms, 800-1000 draws), so their budgets will be
  smaller than in the parks.
- **Props:** catalogue props are ordinary dynamic props (`spawn_runtime_prop`); their count is bounded by the sim
  tick (bodies awake near the skater cost sim time; DownTown is already near the 16.7 ms budget, the parks less).
- **Limits:** retail's Feng Shui meter with **Vita budgets** measured on the device (draws, triangles, awake
  bodies, memory) instead of retail's numbers, so the UI and the rule stay retail's and only the numbers change.
- **UI:** `objectdropper` and `objectquickmenu` through our APT VM, the 58 FE natives, the icons; the help text
  already uses retail's strings.
- **Save:** one edited layout per park (retail has save slots and online sharing; neither applies here), with a
  way back to the Fullpark layout.

## The screen (decoded 29 Oct, D5 research)

Two FE states drive the `ObjectDropper` movie (template `objectdropper/objectdropper.swf`): **0x35, the park editor**
(constructor `825C7720`), opened by flow message **0x79** ("Edit Park", `8270A4B8`), and **0x37, the open-world
dropper** (built in `825E8C40`, vtable `0x82301A58`), opened by flow message **0x7A** (the phone's "Object Dropper",
`8270A5F0`); the flow handler picks between them by message (`8270B3D0`) [code]. The movie asks `IsWorldSpawner`
(= state 0x37 open) and then hides the terrain bar and Properties, shows **"Reset all objects" on Back**, and keeps
Lock Axis and Fine Tune disabled [movie]. Properties (the Quick Menu) is FE state 0x36, movie `objectquickmenu.swf`.

- **The movie takes no keys.** The C++ state handles every key and posts a screen mode (`SetState(n)`, n = the index
  in the name table `0x8302C440`: 0 FreeCam, 1 Catalog, 2 Manipulate, 4 / 5 Next / PreviousCategory, 6 / 7 Next /
  PreviousSubCategory, 8 / 9 Next / PreviousItem, 13 MoveOnLockedAxis, 14 RotateObject, 15 GroupSelect,
  16 HideSelection, 17 ShowSelection, 18 Delete, 19 RefreshFengShui, 20 / 21 Enable / DisableUI, 22 RefreshDuplicate,
  23 RefreshFineTune, 24 RefreshSize, 25 ShowLockAxis, 26 ShowRotate, 27 RefreshInfo, 28 GotoSubCategories,
  29 GotoItems) [data][code]. Its natives are `SkatePark.*` (registrar `825B3CE0`, 59 natives, 46 used by the two
  movies).
- **No item icons:** the catalogue rows are names (`ID_SKATEPARK_OBJECT_<token>` from the model name: terrain the
  second `_` token, props the last); the picture is the item itself, previewed in the world at the cursor while
  browsing (`825D1480` requests it, `825D15C8` puts it in the manipulator) [code]. The 61 `images/objects/` pictures
  in `fedynamic.big` are not used by this screen.
- **Catalogue keys** (park `825CAC18`, open world `825C6C68`): the sub-category list first, then the items. RT / LT
  next / previous super category (wraps, back to the sub list); Up / Down move (wraps; the item index is remembered
  per sub category); A opens the item list, then takes the previewed item in hand (budget `8288EE08`; failure: error
  sound and popup) and goes to Manipulate; B back to the sub list, then out to FreeCam. Park only: X cycles the size
  filter small / medium / large (24), Back opens Properties [code].
- **FreeCam** (park `825CBB18`, open world `825C70E0`): A takes the highlighted object (park: on release; a 200 ms
  hold starts Group Select), with LB held clones it; X opens the catalogue; **Y deletes the highlighted object**
  (park: held, it repeats); B leaves; Back: park Properties, open world the reset popup (`ID_COMMON_WARNING`,
  `ID_DMO_RESET_CONFIRMATION`, Yes / No; Yes resets every DMO, `825C6548` / `825C6638`) [code].
- **Manipulate** (park `825CA178`, open world `825C66D8`): A places (budget `8288EF58`; with LB a copy stays in
  hand), B cancels, Y deletes, X swaps for a catalogue item, L3 resets, RT rotate; park only: RB locked axis, LT fine
  tune, Back Properties (open world: the reset popup) [code].
- **Feng Shui bars:** drawn by the C++ (`825D4828` → `825119C8`) over the movie's `FungShui` (terrain) and
  `FungShuiDynamic` (props) placeholder clips: the used part, then the part to add, colour changing at 0.75 / 0.9;
  the movie only sets alphas and the warning text (`IsFengShuiShowingWarning`) [code][movie].

The natives' return values (the arrays the movie reads), the Quick Menu's states and keys are written up in the D5
research notes kept with the host code (`skate-ui` `object_dropper_host.rs`).

## Snapping (decoded 29 Oct, D6 research)

- **Sockets, not boxes:** every park piece's model carries snap locators (node type 8, group at node +0x64; builder
  `82865230`). Group 0 is a snap point (a full frame); locators sharing a non-zero group form a snap face (up to 4
  corners, framed by the group's first locator, z = outward normal; `82871330`). Each socket carries its piece's
  `AdvanceSnap` cap (180 items: rails, ledges, hubbas, fences, lamps, billboards, trees). Only held park pieces (and
  groups) snap, and only to placed park pieces' sockets (`8287D118`); props never snap [code][data].
- **When:** the editor update (`8288C148`) solves (`82889F58`) the query queued the frame before (`8288A5E0`), queued
  only on frames the cursor moves (move or locked axis). Enable = the Quick Menu's Snapping option (`D+0x1460`,
  default on) [code].
- **Match:** neighbours = the held piece's box grown by 0.75 m; a pair qualifies when the sockets face each other
  (angle between one z and the other's -z under 45 deg, vault `31255295B85ECC39`) and are under 1.5 m apart (vault
  `B0C05776CE3E2630`); score = w x d^2, w = 0.025 for the same AdvanceSnap class, else 1.0; lowest wins. Point to
  point, point to face, face to point, face to face (`8286E880` / `8286EA48` / `8286F018`) [code].
- **Corner vs grid:** corner mode puts the socket on the target face's nearest corner (flush, no gap); grid mode slides
  it on the face in 0.75 m steps inside the face. LT (`GodHand_AdvancedModifier`) up: plain pieces corner mode (grid
  against AdvanceSnap targets), AdvanceSnap pieces grid mode; LT held: plain pieces grid mode, AdvanceSnap pieces do
  not snap [code].
- **Pose:** the held socket goes exactly onto the target, facing it, its in-plane turn rounded to the target's nearest
  90 deg (`82870208`, `828710B8`). Only the drawn pose snaps; the cursor keeps the stick's (no hysteresis: released on
  the first moving frame with no pair). Committed on Rotate, Lock Axis or LT while moving (`8288AB50`); A places at
  the snapped pose; a sound (`0xB28F23C5D0066B02`) when the snap changes [code].
- **World fallback** when no piece qualifies: rays along each held socket's normal from 0.3 m behind to 0.75 m in front
  (`82870388`), up to two sideways refinements of 0.75 m (`828709E8`) [code]; how the refinements combine is unsure.
- **Port:** the bake must extract the sockets per piece; the world fallback needs a ray query.

## The Quick Menu (decoded 29 Oct, D6 research)

- Opens (Back, `825C9F88`) with nothing held or highlighted. Colour, Branding, Style and Options are always enabled;
  Copy needs a selection, Paste a clipboard, Undo / Redo the history (6 snapshots) [code].
- Colour: browsing previews only the selection; Apply (A) needs a colourisable selection (not the spawner); Apply All
  (X) colours every colourisable piece after a Yes / No popup. Style: needs a selection with a style variant; X applies
  it park-wide (unsure). Branding: one park-wide brand (7 entries). Each of the three saves an undo snapshot on entry
  when nothing is held, removed again by B [code].
- Options (`825CE7E0`): internal order Snapping, Invert Y, Invert X, Cursor speed, Collision while the screen shows
  Collision second (a retail quirk in Down); Left / Right toggle; cursor speed 0.5 to 1.5 in 0.1 steps (shown 0-10,
  default 5); saved on exit, forced Snapping and Collision on in challenges [code].
- Copy fills the clipboard and returns; Paste checks the budget (popup when full) and puts the copies in hand; Undo /
  Redo drop what is held, then act [code].

## Open questions (code still to read)

- Where the recipe's `FPSCost` acts (not in the three budgets), and which code writes the meter's fractions
  (`+0x173C..+0x1748`; the budgets above are decoded).
- Field meanings of the three tuning records; the snap rules (basic / advanced); what counts as a valid placement.
- What game modes 4 and 5 are and what byte +323 blocks.
- `world` field `E69A80EE0520102D` (0 in `default`, 1000 StartPark, 2000 DownTown Full).
- What the open world keeps of a drop session (saved, kept until reset, or cleared on leaving).
- How retail treats the skater while editing (frozen, hidden, simulation paused?) and what happens to dynamic props
  placed in the editor when you leave it.
- The phone's "Reset All Props" handler (doc 27 did not find it); the open-world screen's Back resets too (above).

## Proposed steps

| Step | Content |
|---|---|
| D2 | Bake: catalogue pack (item table, meshes, collision, textures, icons, tuning records); per-piece ranges in the park bake |
| D3 | Editor core in skate-sim with tests: modes, free camera, cursor, move / height / rotate / fine tune / lock axis, place, duplicate, delete, reset; the Feng Shui rule decoded and ported |
| D4 | World: edited pieces in render and collision (park: unfuse, re-fuse; districts: instances in the collision window), catalogue props as dynamic props, per-park save, open-world keeping as retail |
| D5 | UI (sub-steps: D5a the retail screen logic in the sim, both variants; D5b the movie, its natives and the Feng Shui bars, the stand-in list removed; D5c the phone rows): dropper and quick-menu movies, natives; the phone rows (Object Dropper / Edit Park, Reset All Props) wired as retail. Catalogue scope (owner, 29 Oct): the open world shows retail's free-play list (about 82 items: find its filter in the code), and a setting adds every other item (as a debug list: pieces retail only allows in custom parks) |
| D6 | D6a snap sockets in the catalogue bake; D6b snapping as retail (sockets, corner / grid, world fallback); D6c the Quick Menu (Properties: options, colour, branding, style, copy / paste, undo / redo); D6d budgets measured on the device, perf on the four parks, group select |
| D7 | Bugs found by the owner on the device, collected until here (for example: the skater should brake and step off the board when the editor opens, no replay from the editor, the editor camera inverted or passing through walls) |

## Owner decisions (28 Oct 2026)

1. **Where:** as retail, the open world and the skate.Parks (the first draft of this survey said parks only; the
   phone code above corrects it).
2. **Unlocks:** every item open until the career (Phase 5) adds unlocks.
3. **Limits:** retail's Feng Shui meter with budgets measured on the Vita.
4. **Scope:** the core editor first (D2-D5), the Properties / quick-menu extras after.
5. **Buttons:** as retail (Select = Properties, Reset All Props on the phone); anything that looks different from
   the game is checked in the code before changing it.
6. **Save:** one saved layout per park, with a way back to the Fullpark layout. **Changed 29 Oct:** no automatic
   save. Parks are saved and loaded later through the Create menu (save the edited park under a name; a list of the
   saved parks to load), as retail's SavePark / EnterSkatePark; until then a park's edits last for the session only.
   In the districts the objects the player placed are temporary: they stay while the player stays in the district
   (teleports within it keep them) and are cleared on a district change and when the game quits (the map's own
   objects are not touched). The D4e per-world save file and
   its restore at load are to be removed.
