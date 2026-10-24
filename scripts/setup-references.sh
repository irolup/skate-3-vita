#!/bin/sh
# Clones the read-only reference repos next to this repo, at the commits this project is built against.
# Existing folders are left untouched.
set -eu
parent=$(cd "$(dirname "$0")/../.." && pwd)

clone() { # url dir commit [ref to fetch first]
    if [ -d "$2" ]; then echo "exists: $2"; return; fi
    git clone -q "$1" "$2"
    if [ $# -ge 4 ]; then git -C "$2" fetch -q origin "$4"; fi
    git -C "$2" checkout -q "$3"
    echo "cloned: $2 @ $3"
}

# skate-3-rust-engine at 0947dea5, a commit of its pull request #52 (the living world: traffic,
# pedestrians, AI skaters). The port and its data converter are built against it. The commit is
# not on a branch, so it is fetched from the pull request.
clone https://github.com/SK8-ENGINE/skate-3-rust-engine "$parent/skate-3-rust-engine" 0947dea5 pull/52/head

mkdir -p "$parent/vita-rust"
clone https://github.com/vita-rust/cargo-vita        "$parent/vita-rust/cargo-vita"        c6bbf22
clone https://github.com/vita-rust/vitasdk-sys       "$parent/vita-rust/vitasdk-sys"       cee132f
clone https://github.com/vita-rust/examples          "$parent/vita-rust/examples"          5ef9e08
clone https://github.com/vita-rust/book              "$parent/vita-rust/book"              fec0a66
clone https://github.com/vita-rust/vita-newlib-shims "$parent/vita-rust/vita-newlib-shims" 013e28c
clone https://github.com/vita-rust/docker            "$parent/vita-rust/docker"            c585981

echo "vitaGL sources (optional): git clone https://github.com/Rinnegatamante/vitaGL ../../vitaGL"
