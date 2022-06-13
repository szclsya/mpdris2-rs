# mpdris2-rs

A lightweight implementation of MPD to D-Bus bridge, which exposes MPD player and playlist information onto [MPRIS2](https://specifications.freedesktop.org/mpris-spec/latest/index.html) interface so other programs can use this generic interface to retrieve MPD's playback state.

## Installation
Clone this repository, and run `cargo build --release`. The MSRV (minimum supported Rust version) is **1.59.0**.

If using Linux with systemd, you can use systemd user service to run mpdris2-rs. Copy the result binary (`target/release/mpdris2-rs`) to `/usr/local/bin`, add [mpdris2-rs.service](misc/mpdris2-rs.service) to `~/.config/systemd/user` and run `systemctl --user enable mpdris2-rs.service`.

## Configuration
Currently the following command line arguments are supported:
- `--host $MPD_HOST` hostname of MPD server
- `--port $MPD_PORT` port of MPD server
- `--no-notification` don't send desktop notification
- `-v` show debug information

## Implementation Status
- [x] Root Interface
- [x] Player control
- [x] Track list (the current playing queue)
- [ ] Playlists
