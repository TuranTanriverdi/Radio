# Ham Radio - Internet Radio Player (Rust)

A lightweight desktop internet-radio player written in Rust. Search thousands of
live stations, play them with a ham-radio-style S-meter, see the current
song, save favourites, and record what you hear.

No account. No ads. No tracking. Just radio.

[Rust](https://img.shields.io/badge/language-Rust-orange)
[License: GPLv3](https://img.shields.io/badge/license-GPLv3-blue)

---

## What it does

Open the app, pick a country + genre (or just type a station name), press
Play. The app probes the stream first, so a dead station tells you
right away instead of freezing the UI.

### Features

**Search & browsing**
- **Global name search** - type `BBC`, press `Enter`, it searches everywhere.
  Country + Genre are *ignored* for name search.
- **Tune All** - browse with the current Country + Genre only
  (e.g. `Turkey - TR` + `rock`). This is the "what's on here?" button.
- **Country / Genre dropdowns** - type to filter, or pick from the list.
  36 countries and 32 genres built in. Country accepts `NL` or
  `Netherlands` / `Turkey - TR` style input.
- Results show `> * [AAC/MP3] Name | bitrate codec | tags`.

**Playback**
- Non-blocking play - probing + connecting run in a background thread,
  the UI never freezes.
- Friendly errors: "This station can't be reached. It may be offline."
  instead of a crash.
- **MP3** via `rodio` / Symphonia, **HE-AAC / AAC** via a dedicated
  `rusty_aac` decoder fallback. HLS (`.m3u8`) streams are rejected with
  a clear message.
- Memory-safe streaming: the MP3 buffer is a capped 4 MiB sliding window,
  not an ever-growing cache. Leave it on all day, memory stays flat.

**Now Playing**
- Shows above the meter: `♪ Artist - Song Title`
- Read from the stream's ICY metadata (`StreamTitle`), polled every 8 s
  on a separate connection - the audio path is untouched.
- If a station sends no metadata: `♪ —`. That is correct, not a bug.

**S-meter / VU**
- Ham-style meter: `S1 S3 S5 S7 S9 +20 +40 +60`, live level + peak.

**Favourites & history**
- `*` to favourite, `x` to delete, `* Favs` to view favourites.
- Favourites + history are stored locally in SQLite
  (`<config>/ham-radio-app/favorites.db`). Nothing leaves your machine.

**Recording**
- `Rec` / `Stop Rec` - records the raw stream as you hear it.
- **File name = song name.** Example: `Sezen Aksu - Gulumse.mp3`
- If the station sends no song: falls back to the station name,
  then to `ham_recording.mp3`.
- **Auto-split:** if the song changes while recording, the current file
  closes and a new file starts with the new song name - one file per song.
- Extension follows codec: `.mp3` for MP3, `.aac` for AAC.
- File names are sanitised for Windows (`\ / : * ? " < > |` -> `-`).

**Keyboard & hotkeys**
- `Space` - Play / Stop
- `+` / `-`, Up / Down - Volume
- `Ctrl+Alt+P` - Play/Pause (global), `Ctrl+Alt+N` - Next favourite,
  `Ctrl+Alt+B` - Previous favourite, `Ctrl+Alt+Up/Down` - Volume

---

## Quick start

### Requirements
- Rust + Cargo (stable, 2021 edition)
- Windows is the primary target. Linux / macOS should work with the same
  commands, audio via the system default device.

### Run
```powershell
cargo run --release -p ham-radio-gui -- --country "Turkey - TR" --genre rock --limit 30
```

### CLI options
| Flag | Default | Meaning |
|---|---|---|
| `--country` | `NL` | e.g. `NL`, `Turkey`, `Turkey - TR` |
| `--genre` | `classical` | e.g. `rock`, `jazz`, `news` |
| `--limit` | `20` | How many stations to fetch per search |

### Build only
```powershell
cargo build --release
# binary: target/release/ham-radio-gui.exe
```

---

## File structure - what is what

This is a Cargo workspace. One workspace, several small crates. Each crate
does one job, so you can change playback without touching the UI.

```
ham-radio-app/
├── Cargo.toml                  # Workspace: members + shared version/edition
├── README.md                   # This file
├── LICENSE                     # GPLv3 text (see License below)
├── .gitignore                  # target/, recordings, *.db - see below
│
├── app-gui/
│   ├── Cargo.toml              # Binary crate `ham-radio-gui` (eframe/egui, clap)
│   └── src/
│       └── main.rs             # ALL of the UI. Window, search boxes,
│                               # station list, VU meter, Now Playing,
│                               # Rec button, hotkeys, keyboard handling.
│                               # Current: V12 (Rec = song name, auto-split)
│                               # + Now Playing poller (8 s).
│
└── crates/
    ├── radio-api/
    │   ├── Cargo.toml          # reqwest + serde
    │   └── src/lib.rs          # RadioBrowser API client.
    │                           # Station struct, SearchFilters,
    │                           # search(name, country, genre, limit).
    │                           # Talks to all.api.radio-browser.info
    │                           # with fallbacks (de1, nl1, fr1, at1).
    │
    ├── radio-core/
    │   ├── Cargo.toml
    │   └── src/lib.rs          # Glue: RadioAppCore.
    │                           # Holds api + player + storage,
    │                           # current station, play/stop/volume,
    │                           # record start/stop, sleep timer,
    │                           # favourites/history passthrough.
    │                           # The GUI only talks to this crate.
    │
    ├── radio-player/
    │   ├── Cargo.toml          # rodio (symphonia-all), ureq, rusty_aac
    │   └── src/lib.rs          # ALL audio. Player, Probe, VU meter,
    │                           # SeekableHttpResponse (capped 4 MiB
    │                           # sliding window, V13 memory fix),
    │                           # rusty_aac fallback for HE-AAC,
    │                           # fetch_icy_title() for Now Playing,
    │                           # recording (raw stream -> file).
    │                           # Current: V13.
    │
    ├── radio-storage/
    │   ├── Cargo.toml          # rusqlite
    │   └── src/lib.rs          # SQLite: favourites + history tables,
    │                           # migrations for old DB schemas.
    │                           # DB lives in <config>/ham-radio-app/
    │
    └── radio-tray/
        ├── Cargo.toml          # global-hotkey
        └── src/lib.rs          # Global hotkeys + tray stub.
                               # Ctrl+Alt+P/N/B/Up/Down.
                               # Tray menu is a stub (logs only).
```

### Data flow

```
GUI (app-gui/src/main.rs)
  -> RadioAppCore (radio-core)
       -> RadioBrowserClient (radio-api)  : search stations
       -> Player (radio-player)           : probe + decode + play + record
       -> Storage (radio-storage)         : favourites / history
       -> TrayHotkeyManager (radio-tray)  : global hotkeys
```

If you want to change *how it looks*, edit only `app-gui/src/main.rs`.
If you want to change *how it sounds*, edit only
`crates/radio-player/src/lib.rs`. The other crates don't need to change.

---

## Changing the song-refresh rate

In `app-gui/src/main.rs`, in `play_station_nonblocking()`:

```rust
std::thread::spawn(move || {
    loop {
        if let Some(title) = radio_player_crate::fetch_icy_title(&stream_url) {
            if song_tx.send(title).is_err() { break; }
        }
        std::thread::sleep(Duration::from_secs(8)); // <- change this
    }
});
```

Change `8` to e.g. `3` for faster updates, `15` for slower. Don't go below
3 s - every poll opens a new HTTP connection to the station.

---

## Data sources & attribution - read this before publishing

### 1. Station list: RadioBrowser - YES, you must credit it

This app does **not** ship its own station list. Every search goes live to:

- **RadioBrowser** - https://www.radio-browser.info / `all.api.radio-browser.info`
- A community-maintained, open database of internet radio stations.

You must say that in the README / About box, and you must **not** claim
the list is yours. The attribution in this README ("Station list provided
by RadioBrowser") is the minimum. Keep the `User-Agent: ham-radio-app/0.1.0`
in `radio-api` - that is how RadioBrowser identifies clients.

Do **not** copy RadioBrowser's database into your repo and re-publish it
as your own. Query the API at runtime, as this code does.

### 2. Audio streams & music: you don't own them

The app plays streams owned by the stations. Songs, jingles, news and
other audio remain copyright of the stations / rights holders.

- This repo contains **no audio, no station streams, no music**.
- Recording is for **personal use** - the file is saved on the user's
  own machine. Do not upload recordings, do not redistribute them.
- If a station asks not to be listed / played, respect that (RadioBrowser
  has a takedown / `hidebroken` mechanism).

### 3. Dependencies

The app uses open-source crates (egui/eframe, rodio/Symphonia,
rusty_aac, reqwest, rusqlite, etc.). Each keeps its own license
(MIT / Apache-2.0 / etc.). Those licenses are compatible with GPLv3,
but they still apply to those crates - see `Cargo.lock` / crates.io
for the full list.

---

## License

This project is licensed under the **GNU General Public License v3.0
(GPLv3)** - see `LICENSE`.

What that means in plain language:

- Anyone may use, study, modify and share this code.
- If someone distributes this app (or a modified version), they must
  also distribute the source code under the **same** GPLv3 license.
- GPLv3 does **not** give anyone rights to the RadioBrowser data or
  to the audio streams / music - those have their own owners (see above).

To apply GPLv3 on GitHub: put the full GPLv3 text in a file named
`LICENSE` in the repo root, and keep the `License` section above in
this README. If you create the repo via github.com, you can also pick
"GNU General Public License v3.0" in the "Add a license" dropdown -
it creates the same `LICENSE` file for you.

> If you later want a more permissive license (e.g. MIT - "do anything,
> no need to share source"), you would need to re-license. GPLv3 is the
> stricter, copyleft choice: it keeps forks open too.

---

## .gitignore - what NOT to commit

Don't commit build output, local databases, or recordings:

```gitignore
/target/
*.db
*.mp3
*.aac
ham_recording*.mp3
*.log
```

---

## Contributing / issues

Issues and PRs welcome. Please keep changes small and scoped:

- UI only -> `app-gui/src/main.rs`
- Playback only -> `crates/radio-player/src/lib.rs`
- Search / API only -> `crates/radio-api/src/lib.rs`

If you add a feature, update this README's feature list in the same PR.
