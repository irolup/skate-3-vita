# skate.Park saving, loading and setup: survey

Owner's request (31 Oct): after the Object Dropper, save and load skate.parks and change the menus for it as retail
does. This survey says what retail has, what the Vita build has already, and the steps to build it. Research only:
nothing here is built yet. Retail references are addresses in the TU3 executable (Ghidra) and the disc's data.

## What retail has

### The Create tab

The pause menu's crossbar (`menu-tables.json` `modes`, retail table `0x82FCBB54`) has a Create tab whose rows depend on
where the player is:

| Mode | Create rows |
|---|---|
| FreePlayPark (free skate in a skate.park) | Save Park, skate.Park, Replay Editor, skate.Reel, Rate a Park, Flag Park, Project 10 |
| CareerPark (career, in a skate.park) | the same |
| FreePlay (free skate in a district) | Replay Editor, skate.Reel, skate.Park |
| Career (a district) | Replay Editor, skate.Reel, skate.Park, Project 10 |

`skate.Park` (item `EnterSkatePark`, sub option list 0) opens a sub list: **New Park**, **My Parks**, Community Parks,
Tutorial. `Save Park` (`SavePark`) saves the park being edited. Upload, Rate, Flag and Community Parks are online. The
debug items `LoadParkFromFile` / `SaveParkToFile` / Maya import and export are not shown in any mode.

### Park Setup (New Park)

FE state `0x3A` (vtable `0x82301BD0`; notes `~/tools/re-out/notes/sp-parksetup.md`), movie `skatepark/parksetup`
(`ID_CROSSBAR_PARK_SETUP`): two selectors, **Lot** and **Template**, a thumbnail and a bar. The movie takes no keys:

| Key | Retail (`825D28C0`) |
|---|---|
| Up / Down | the active selector (lot, template) |
| Left / Right | the selector's choice; a lot change resets the template |
| A | lot unlocked: if the park has unsaved edits, popup `ID_SKATEPARK_DIRTY_TITLE` / `ID_SKATEPARK_DIRTY_DESC` (No / Yes); then the template's world loads (flow message `0x76`). Locked: a DLC or a career dialog |
| B | back |

Natives (`SkatePark` object): `ParkSetup_GetSelectedName(0 lot / 1 template)` (the world's display name),
`ParkSetup_GetSelectedThumbnail` (`fe\source\images\skatepark\<thumb>`), `ParkSetup_GetSelectedBar` (the active
selector), `ParkSetup_IsSelectedParkUnlocked` (the lot's unlock key).

The lists come from the vault's `world` rows: the lots are the children of `default_skateparks` with a park field, the
templates each lot's children with a thumbnail:

| Lot | Templates | Layouts |
|---|---|---|
| `dist_megapark` (Stadium) | Empty, Full | (none), `MegaPark_Fullpark` |
| `dist_startpark` | Full, Empty | `Starter_Fullpark`, (none) |
| `dist_downtownpark` | Full, Empty | `Downtown_Fullpark`, (none) |
| `dist_industrialpark` | Empty, Full | `Industrial_Empty`, `Industrial_Fullpark` |

(retail sorts the lots, comparator `825D1AD0` not read; an Empty without a file is the bare lot.)

### Save Park

Crossbar item `0x12` posts flow message `0x78` (`8270A278`): the keyboard, title `ID_SKATEPARK_NAME_PARK`, the park's
current name, 16 characters at most. An existing name asks `ID_COMMON_WARNING_TITLE` / `ID_SKATEPARK_ALREADY_EXISTS`
(Yes overwrites, No returns to the keyboard); a new name adds the park. Retail keeps the parks in its save data: a list
of 224-byte records (id, name, flags) with their layouts, written by the save manager.

### My Parks

skate.Park's second row opens the shared browser (FE state `0x3B`, vtable `0x82304260`, movie `browser/browser`, object
`CommonBrowser`, 30 natives) on parks: top tabs **My Parks**, Downloaded, Community; under My Parks **Saved Parks** and
Shared Parks (owner's retail description; tab table `0x83027AB0`). A grid of eight thumbnails a page, each park's picture
from the memory card, and an item panel (date, creator, tags...). Keys: LB / RB the top tabs, LT / RT the bottom ones, the
d-pad the thumbnails (Up / Down four), B back; A selects and Y deletes (the movie's). Selecting a saved park loads it
(flow message `0x75`), asking `ID_SKATEPARK_LOAD_PARK_CONFIRM` (Yes / No) when the park being skated has unsaved edits;
Delete removes it through the save manager. Community and Downloaded are online. Notes: `sp-parksetup.md`.

### The park file

A `.park` file (upstream `park_layout.parse`, research b425): header (version, block lengths, origin world hash), the
static pieces (position, rotation, recipe id, tint), the movable props, then the editor tail whose first matrix is the
player start (where the spawner stands). Version 10 on the disc.

## What the Vita build has

- The Object Dropper edits a park in the session: pieces moved, coloured, removed, dropped; the spawner; teleporters.
  Nothing is saved (c778810 took the old per-world save file out; `object_dropper/save.rs` still holds its diff format).
- Each park map carries its Fullpark layout baked in (Phase 2): the fused collision and the pieces' draws. A layout
  piece can be moved or hidden at run time; new pieces are dropped items (collision layer, budgets).
- The pause menu's Create tab shows only the Replay Editor (`vita_menus.rs`).
- The menu stage runs retail movies with natives; the `SkatePark` object exists for the dropper's natives.

## Proposed design

Retail's menus and flow, with the Vita's storage:

1. **Create tab as retail.** In a park: Save Park, skate.Park (New Park, My Parks), Replay Editor. In a district:
   Replay Editor, skate.Park. The online rows stay out.
2. **New Park: the Park Setup screen** (retail movie and natives). Lots: the four parks with a layout. Templates: each
   lot's flagged rows (Full, Empty). Choosing one loads the park with that layout and opens the park editor.
3. **Any layout on a baked park.** A `.park` file is applied to the park's baked Fullpark: a layout piece that matches a
   Fullpark piece (same item and pose) stays; a Fullpark piece the layout lacks is removed; any other piece is dropped
   (budgets as for the dropper); the props are dropped; the start places the spawner. Empty = every piece removed.
4. **Save Park** as retail (keyboard, 16 characters, the overwrite question) writes the edited park as a retail `.park`
   file (version 10, the spawner as the start) under `ux0:data/SKATEVITA/parks/`, with an index (name, lot, date) and
   a small picture taken from the screen (retail's park pictures).
5. **My Parks** runs the retail browser movie with our `CommonBrowser` natives (the saved parks only); Select loads the
   lot and applies the park (step 3), Delete asks first.

## Open questions (Ghidra next)

- Done: Park Setup (state, keys, natives, lists), Save Park (keyboard, overwrite), My Parks is the browser.
- Done also: the browser's C++ side (natives, keys, park provider, popups).
- Done (S5): the lot order is the `world` field `E69A80EE0520102D`, smallest first (`825D1AD0`): Starter 1000, Industrial
  2000, Downtown 3000, the Stadium 4000. Flow `0x76` (`827099F0`) keeps the template world and runs the park load states
  (`8270A1F8` / `8270A088`); neither opens the editor (a flag it carries, `+0xB8`, is not traced).
- Done: Delete asks first (`82523BC0`: `ID_COMMON_WARNING_TITLE` / `ID_MEMCARD_PROMPT_DELETE_SKATEPARK`, No / Yes); saving
  and deleting show the autosave layer's save wheel until done (owner); a new park starts on the board, no editor (owner).
- Still open: how many parks retail keeps; the picture's size; the save wheel's style retail passes.

## Steps

| Step | What | Check |
|---|---|---|
| S1 | Ghidra: ParkSetup natives, Save Park and My Parks flows (notes `~/tools/re-out/notes/sp-parksetup.md`) | done, but the browser's C++ side |
| S2 | `.park` read / write in the sim; apply a layout to a baked park (step 3); tests on the stock layouts (Empty, Full, progression) | host tests |
| S3 | Create tab rows; Save Park (keyboard, file, index) | device |
| S4 | My Parks (list, load, delete) | device |
| S5 | New Park: Park Setup screen (movie, natives, lot / template), load (done 2026-11-02; the editor is not opened, see below) | device |
