# Gel-O

Gel-O is a library that provides a `std::iter::Iterator` interface over Linux `InputEvent`s. You can map, filter and loop over input events without dealing with the low-level details of obtaining those events.

Gel-O is so named for examples/delay.rs, which delays all user input, making a computer feel like Jell-O.

# Stability

Gel-O is experimental. Expect many breaking changes to come.

## Similar crates

- [evdev](https://github.com/emberian/evdev)
    - A pure-rust Iterator over `InputEvents`, with no support for writing to devices (simulating user input).
- [evdev-rs](https://github.com/ndesh26/evdev-rs)
    - Low-level safe bindings to [libevdev](https://www.freedesktop.org/wiki/Software/libevdev/). Gel-O is built atop evdev-rs.

## Requirements

- Linux
    - epoll (Linux 2.6.27+)
    - kernel supporting evdev (~ 2.4+)
- Rust toolchain
    - version 1.40 or higher
    - cargo
- Transitive dependencies
    - C toolchain
    - autoconf and libtool
        - `#apt install autoconf libtool`
        - `#yum install autoconf libtool`
        - `#pacman -S autoconf libtool`

macOS and Windows are not supported and support is not planned. BSD support may be easy to add, as evdev was recently added to FreeBSD.

## FAQ

- How does it work?
    - Gel-O monitors device files in `/dev/input`, reading `InputEvents` lazily only when user code calls `.next()`.
- Does it work when using the [Wayland Display Server Protocol](https://wayland.freedesktop.org/)?
    - Yes. Gel-O works everywhere linux does - on Xorg, Wayland, and even the Linux virtual terminal
- What is the computational overhead of Gel-O?
    - Gel-O is pretty leightweight. Most of the examples use around 1MB of RAM. Mouse movement is smooth on low-power devices (such as Raspberry Pi) due to efficient epoll-based architecture. On raspberry pi 3, expect Gel-O to use ~2% CPU during rapid mouse movement and 0% otherwise.
- What devices does Gel-O work with?
    - Gel-O works with every input device Linux does because Gel-O operates just above the driver level. Gel-O has been tested specifically with mice, keyboards, power buttons and gamepads.
- What happens if I unplug my device and plug it back in?
    - Gel-O detects new devices being plugged in, and starts monitoring them. Unplugging and plugging in might mean a few events are lost, but everything will continue smoothly after that loss.

## Limitations

- Requires read/write access to files in `/dev/input` and the file `/dev/uinput`
    - This can be acomplished by running as root (sudo)

## Developing

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
make changes to source files. Before committing, run `checks.sh` (this checks formatting and for compiler warnings)
## TODO
- [ ] add tests