# skate-3-vita

A PS Vita Skate 3 port of the [skate-3-rust-engine](https://github.com/SK8-ENGINE/skate-3-rust-engine) from the Xbox 360 Skate 3.

Not in this beta yet: the career (challenges, progression, unlocks), online play.

**Known issues:** in DownTown with traffic, AI skaters and pedestrians all on, the simulation runs slightly over its time budget and drops some ticks. Graphics are reduced to fit the Vita: low-resolution textures, no shadows, baked lighting.

The controls are in [docs/controls.md](docs/controls.md).

## Making your own game data

The game needs files made from **your own** Xbox 360 copy of Skate 3. It currently only works on Linux. You'll end up with a folder of `.pak` files that you copy onto your console.

### What you need

- Your own Xbox 360 disc of Skate 3, as an ISO file.
- A few tools. On Linux, open a terminal and run:

  ```sh
  sudo apt install git curl unzip build-essential cmake python3 python3-numpy python3-pil
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```

  The second line installs [Rust](https://rustup.rs). Close the terminal and open a new one afterwards.

### Step 1 : download this project and set up the folders

```sh
mkdir skate && cd skate
git clone https://github.com/irolup/skate-3-vita skate_vita
cd skate_vita
scripts/setup-references.sh
mkdir -p ../game_data
```

`setup-references.sh` downloads the reference project this port's game logic is translated from, at the exact version this port is built against: you need its converter for your disc. It also downloads a few Vita development tools you can ignore. You should now have:

```
skate/
├── skate_vita/
├── skate-3-rust-engine/
└── game_data/
```

Run every command below from inside `skate_vita/`, in the same terminal window: later steps reuse `INSTALL`, which step 3 sets.

### Step 2 : extract your ISO

Build the extraction tool, then extract your ISO with it. Replace `/path/to/Skate 3.iso` with your own file, and keep the quotes:

```sh
git clone https://github.com/XboxDev/extract-xiso ../game_data/tools/extract-xiso
cmake -S ../game_data/tools/extract-xiso -B ../game_data/tools/extract-xiso/build
cmake --build ../game_data/tools/extract-xiso/build
../game_data/tools/extract-xiso/build/extract-xiso -x "/path/to/Skate 3.iso" -d ../game_data/skate3_x360
```

CMake may print a "deprecated" warning while it builds; that's harmless. Extracting unpacks about 6 GB of disc files into `../game_data/skate3_x360/`. You should see a `default.xex` file in it.

### Step 3 : convert the disc into Vita-ready data

The converter is a Python tool from the reference project. It decodes the game's sounds with vgmstream, and because it was written for Windows it would download the Windows version. Give it the Linux one first:

```sh
V=../game_data/prepared/tools/vgmstream-cli
mkdir -p $V
curl -L -o $V/linux.zip https://github.com/vgmstream/vgmstream/releases/download/r2117/vgmstream-linux.zip
unzip -o $V/linux.zip -d $V && mv $V/vgmstream-cli $V/vgmstream-cli.exe && chmod +x $V/vgmstream-cli.exe && touch $V/.complete
```

(The `.exe` ending is only the name the converter looks for. It's the Linux program.) Then run the converter:

```sh
python3 ../skate-3-rust-engine/tools/prepare_assets.py --game-root ../game_data/skate3_x360 --output ../game_data/prepared --game-exe /bin/true
```

It prints its progress as it goes. `--game-exe /bin/true` skips a few extra checks that need a Windows program; the Vita version doesn't use what they check. When it's done, save where it put everything:

```sh
INSTALL=../game_data/prepared/$(python3 -c "import json; print(json.load(open('../game_data/prepared/installation.json'))['directory'])")
echo $INSTALL
```

The retail menus also need the menu tables, which are read from your disc's `default.xex`. This builds a small tool from the reference project of step 1 and runs it (the first time, Rust downloads what it needs, which takes a minute):

```sh
cargo run --release --manifest-path tools/menu-tables/Cargo.toml -- ../game_data/skate3_x360/default.xex $INSTALL/assets/private/menu-tables.json
```

It should end with a line like `menu-tables.json: 47 pause items, 35 settings rows, 8 modes`.

### Step 4 : rebuild the game's menu screens

The main and pause menus, Game Settings, the replay editor and their popups are the game's own front-end movies. The first line exports them from your disc with the reference project's exporter. The loop then rebuilds two more screens: the manual meter and the trick analyzer. Copy the whole block at once:

```sh
python3 tools/prepare_menu_screens.py --game ../game_data/skate3_x360 --assets $INSTALL/assets --work ../game_data/menu-movies-work
C=$INSTALL/assets/private/stock/skater-collections.json
for s in hud2/manualmeter hud2/trickanalyser hud2/hudminimap; do
    python3 tools/prepare_ui_screen.py --game ../game_data/skate3_x360 --collections $C --output ../game_data/ui-screens --bundle data/fe/source/screens/$s
    python3 tools/link_ui_screen.py --game ../game_data/skate3_x360 --collections $C --output ../game_data/ui-screens --screen ${s##*/}
done
```

### Step 5 : build vita-bake and bake the packs

`vita-bake` is this repo's own tool. The first line builds it (the first time, Rust downloads what it needs, which takes a few minutes); the `$B` lines then bake each pack. They're independent, so if one fails you can fix it and re-run just that one:

```sh
cargo build --release -p vita-bake
OUT=../game_data/SKATEVITA
B=target/release/vita-bake
$B sim      $INSTALL/assets $OUT
$B skater   $INSTALL/assets/private/skater.glb $OUT
$B ui       $INSTALL/assets $OUT ../game_data/ui-screens
$B maps     $INSTALL $OUT
$B district $INSTALL $OUT
$B props    $INSTALL $OUT
$B living   $INSTALL $OUT
$B audio    $INSTALL/assets/private/audio $OUT
$B ambience $INSTALL/assets/private/audio $OUT
$B trickguide $INSTALL $OUT
```

| Line | What it makes |
|---|---|
| `sim` | the simulation data |
| `skater` | the skater and board model |
| `ui` | the in-game HUD and the menus from step 4 |
| `maps` | the 7 small skateparks |
| `district` | the 3 big campaign districts: University, DownTown and Industrial. It's the slowest line and makes most of the data |
| `props` | the movable objects in the districts |
| `living` | traffic, pedestrians and AI skaters |
| `audio` | the board's own sounds |
| `ambience` | the ambient city sounds |
| `trickguide` | the Trick Guide's 3D demo: its set and the skater's trick animations |

When it's done, `../game_data/SKATEVITA` holds about 1 GB of `.pak` files (and matching `.idx` files). That's your complete game data.

### Step 6 : copy it to your Vita

Copy the whole `../game_data/SKATEVITA` folder onto your console at `ux0:data/SKATEVITA/`. Install the game's VPK if you haven't already.

## Building from source

To build it yourself you need Linux, [VitaSDK](https://vitasdk.org) with `VITASDK` set (it provides vitaGL, vitashark and mathneon), Rust nightly with `rust-src`, and [cargo-vita](https://github.com/vita-rust/cargo-vita) with the binary-FTP patch in [docs/patches](docs/patches) (stock cargo-vita 0.2.2 corrupts uploads). [docs/toolchain.md](docs/toolchain.md) has the details.

- `scripts/vita-run.py` builds the game, installs it over the app on a Vita running [vitacompanion](https://github.com/devnoname120/vitacompanion) (with `VITA_IP` set) and shows its log.
- For a VPK, run `cargo vita build vpk -- --release -p skate-vita` in the environment that `sdk_env()` in `scripts/vita-test.py` sets up (VitaSDK on `PATH`, and the `list-ilp` scheduler flag the simulation's speed relies on).

Run `scripts/install-hooks.sh` once after cloning: it installs a check that stops any commit containing game data.

## AI usage

AI coding tools were used to build this port. The knowledge of how Skate 3 works comes from [skate-3-rust-engine](https://github.com/SK8-ENGINE/skate-3-rust-engine) and the reverse-engineering work behind it.

## License

This project is under the [GNU General Public License v3.0](LICENSE) (GPL-3.0-only), like [skate-3-rust-engine](https://github.com/SK8-ENGINE/skate-3-rust-engine), which most of its game logic is ported from. [PROVENANCE.md](PROVENANCE.md) lists what was taken from it and from the other projects this one builds on.

Skate and Skate 3 belong to Electronic Arts. This is an unofficial fan project, not affiliated with or endorsed by EA, and it contains none of the game's files.
