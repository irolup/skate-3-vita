# Controls and menus

Skate 3's Xbox 360 controls, by physical position. Everything after the conversion of the Vita's pad and touch input to an Xbox state is the unchanged game logic, so the tricks, grinds and manuals work as in Skate 3.

## In play

| Vita | Xbox 360 | Used for |
|---|---|---|
| Left stick | Left stick | steer, lean, kick-turn, powerslide (full sideways), body spin in the air, grind balance, walking off the board |
| Right stick | Right stick | **flick-it tricks** (pull back and flick), crouch, manuals, board adjust in the air, grind up/down |
| Cross | A | push (hold for speed); recover after a bail; sprint off the board |
| Square | X | mongo push; jump off the board; plants |
| Circle | B | brake, powerslide; dismount |
| Triangle | Y | get off / on the board |
| L | LT | left grab, crouch (a digital press: full strength) |
| R | RT | right grab, throw the board (off the board) |
| Rear touch, left half (held) | LB | modifier; session markers |
| Rear touch, right half (held) | RB | grab the world: handplants, object grabs |
| Front touch, left / right half | L3 / R3 | bail chord (LT + RT + L3 + R3) |
| D-pad | D-pad | up/down as is; left/right as LB/RB if the setting is on |
| Start | Start | pause menu |
| Select + Start | | debug overlay (fps, tick time, heap, state) |

The rear touch zones and the front touch zones can be turned off, and the d-pad can stand in for LB/RB, in **Settings** (accidental rear-touch presses are common).

## Menus

D-pad up/down moves, left/right changes a value, Cross chooses, Circle goes back.

**Main menu:** Skate, Park (Course, StartPark, MegaPark, BlackBoxPark, IndustrialSkatePark, DownTownSkatePark, MaloofMoneyCup, SkateSchool), Difficulty (Easy, Normal, Hardcore), Controls, Stick calibration, Settings, Quit. A park marked `*` loads when you press Skate.

**Pause menu (Start while skating):** Resume, Respawn (back to the park's spawn), Replay Editor (the instant replay or a saved replay), Park (choose, then press Cross to load it), Difficulty (applies at once), Controls, Settings, Main menu, Quit.

**Settings:** rear touch as LB/RB, d-pad left/right as LB/RB, front touch as L3/R3, volume (0 to 10), debug overlay. Saved on every change in `ux0:data/SKATEVITA/settings.bin`.

**Physics** (Skate menu and the main/pause menus): *Fast* (default) solves the board and body with 25 constraint iterations, *Retail* with the game's 50. Retail is the exact retail behaviour (the wheels settle a little less and the pop is about 1 cm higher) but its slowest ticks do not fit the Vita's 60 Hz budget, so the game can slow down in places. Both are checked bit for bit against the reference.

**Audio** (Options menu, Settings on the plain menus): *Retail* (default) plays the game's own board sounds from `sound.pak` (vita-bake audio): about 50 MB of memory, with the game's own packs read from the memory card to make room; with `ambience.pak` and `world.pak` also uploaded it plays the world around you too (the ambience of each area with its transitions, local sounds near equipment, trees and crowds, and the reverb of the place you are in), streamed from the card. *Synth* plays the light procedural sounds instead, and they are what plays when `sound.pak` is missing. The retail sounds load a few seconds after launch (or at once when chosen); back to Synth mutes them, and their memory is freed at the next launch.

**Stick calibration:** the raw and converted stick positions, the magnitude reached in each of 32 directions (every direction should reach OK, converted magnitude 0.95 or more), the touch zones, the Xbox buttons being sent and the tricks recognized. Start goes back.

## Replay editor (P1)

Tap **Select** while skating (or pause menu > Replay Editor) to watch the last minute. The retail replay editor's controls, in the Xbox layout the game uses (see the table above):

| Xbox 360 | Vita | Instant replay | Edit timeline |
|---|---|---|---|
| RB | rear touch, right half | play / pause | play / pause |
| LT / RT | L / R | wind back / forward | wind back / forward (inside the In and Out points) |
| D-pad left / right | D-pad | skip to the start / end | skip to the previous / next marker (In and Out points when none) |
| D-pad down / up | D-pad | | place a clip marker / remove the marker or camera key under the playhead |
| LB + left / right stick | rear touch left + stick | | drag the In / Out point |
| right stick / left stick | sticks | orbit the camera / zoom | |
| A | Cross | Edit timeline | Edit clip (camera key: edit it) |
| X | Square | Take Photo (`ux0:data/SKATEVITA/photos/`) | Take Photo |
| B | Circle | Finished: Upload Video (not on the Vita), Save Replay (`ux0:data/SKATEVITA/replays/`), Exit | Finished |
| L3 | front touch, left half | hide the editor | hide the editor |
| Start | Start | help | help |

**Edit clip** (one clip: between two markers): Game, Follow, Tripod and Free cameras, their Advanced versions (each Commit sets a camera key; the camera blends between keys), Speed (5 settings, 3 is normal) and Shake (Off to High). Camera screens: right stick rotates, left stick moves in and out, D-pad up/down moves up and down (or the aim on the skater), left/right zooms; A commits, B goes back. On the timeline each clip shows in its camera's colour (Game yellow, Follow blue, Tripod grey, Free violet). Saved replays open from the pause menu (Replay Editor), their park loading first.

## HUD

Top left: the current trick (with Fakie / Switch / Nollie in front), then the sequence score with its multiplier and CLEAN or SKETCHY. Top right: the line score and its timer bar. Bottom left: the session total.
