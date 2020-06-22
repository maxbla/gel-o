# Gel-O

Gel-O is a library for manipulating user-input in Linux.

Gel-O can make your computer feel like Jell-O, by delaying all user input by a configurable number of milliseconds (and in various other ways).

# Stability

Gel-O is experimental. Expect many breaking changes to come.

## Why?

For fun, and to get my feet wet.

## Requirements

- Linux
    - epoll (Linux 2.6.27+)
    - evdev kernel (approximately Linux 2.4+)
- Rust toolchain
    - version 1.34 or higher
- Transitive dependencies
    - C toolchain
    - autoconf and libtool
        - `#apt install autoconf libtool`
        - `#yum install autoconf libtool`
        - `#pacman -S autoconf libtool`

macOs and Windows are not supported at this time.

BSD support may be easy to add, as evdev was recently added to some BSD kernels

## Features

- Configurable delay
- Works on Xorg, Wayland, and the Linux virtual terminal
- Runs smoothly on low power devices like raspberry pi due to efficient epoll-based architecture
- Works with every input device Linux does including mice, keyboards, power buttons, gamepads, flight sticks and more

## Anti-features

- Requires read/write access to files in /dev/input, /dev/uinput

## How to use

Download the source
```
git clone [this repo]
```
Compile the source. You need a rust toolchain and cargo.
```
cargo build --release --example delay
```
Run the produced binary with root privledges
```
sudo ./target/release/examples/delay [number of ms to delay]
```

## TODO
- [x] monitor filesystem for new devices, and add delay to them too (probably using inotify)
- [ ] add tests
- [ ] Create serializable event struct that sidesteps ownership issues with libevdev