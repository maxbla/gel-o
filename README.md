# Gel-O

Gel-o makes your computer feel like Jell-O, by delaying all user input by a configurable number of milliseconds

## Why?

For fun, and to get my feet wet.

## Requirements

- Linux
    - epoll added in Linux kernel 2.6.27
    - the evdev kernel module added around Linux kernel 2.4

macOs and Windows are not supported at this time.

BSD support may be easy to add, as evdev was recently added to some BSD kernels

## How to use

Download the source
```
git clone [this repo]
```
Compile the source. You need a rust toolchain and cargo.
```
cargo build --release
```
Run the produced binary with root privledges
```
sudo ./target/release/gel-o [number of ms to delay]
```