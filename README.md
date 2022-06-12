# mpdris2-rs

A lightweight implementation of MPD to D-Bus bridge, which exposes MPD player and playlist information onto [MPRIS2](https://specifications.freedesktop.org/mpris-spec/latest/index.html) interface so other programs can use this generic interface to retrieve MPD's playback state.

## Installation
Clone this repository, and use `cargo build --release`. The MSRV (minimum supported Rust version) is **1.59.0**.

## Configuration
Currently the following command line arguments are supported:
- `--host $MPD_HOST` hostname of MPD server
- `--port $MPD_PORT` port of MPD server
- `--no-notification` don't send desktop notification
- `-v` show debug information
