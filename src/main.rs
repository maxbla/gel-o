use evdev_rs::{Device, InputEvent, UInputDevice};
use std::collections::LinkedList;
use std::convert::TryInto;
use std::ffi::OsStr;
use std::fs::{read_dir, File};
use std::io;
use std::os::unix::{ffi::OsStrExt, fs::FileTypeExt, io::AsRawFd};
use std::thread::sleep;
use std::time::{Duration, Instant};

fn get_all_device_files() -> io::Result<Vec<File>> {
    let mut res = Vec::new();
    for entry in read_dir("/dev/input")? {
        let entry = entry?;
        // /dev/input files are character devices
        if !entry.file_type()?.is_char_device() {
            continue;
        }

        // these files don't play nice with libevdev, not sure why
        // see: https://askubuntu.com/questions/1043832/difference-between-dev-input-mouse0-and-dev-input-mice
        entry.path().file_name().map(|file_name| -> io::Result<()> {
            let bytes = file_name.as_bytes();
            // skip filenames matching "mouse.* or mice"
            if bytes == OsStr::new("mice").as_bytes()
                || &bytes[0..=4] == OsStr::new("mouse").as_bytes()
            {
                return Ok(());
            }
            println!("opening: {:?}", entry.path());
            let file = File::open(entry.path())?;
            res.push(file);
            Ok(())
        });
    }
    Ok(res)
}

fn main() -> io::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let ms_delay: u32 = args.get(1).map(|arg| arg.parse().unwrap()).unwrap_or(60);

    //TODO: remove this hack
    //wait for enter key to be released after starting
    sleep(Duration::from_millis(500));

    println!("opening /dev/input files...");
    let device_files = get_all_device_files()?;

    println!("initalizing epoll...");
    let epoll_fd = epoll::create(true)?;
    // add file descriptors to epoll
    for (file_idx, file) in device_files.iter().enumerate() {
        let device_fd = file.as_raw_fd();
        let epoll_event = epoll::Event::new(epoll::Events::EPOLLIN, file_idx as u64);
        epoll::ctl(
            epoll_fd,
            epoll::ControlOptions::EPOLL_CTL_ADD,
            device_fd,
            epoll_event,
        )?;
    }

    let devices = device_files
        .into_iter()
        .map(|file| {
            println!("setting up device for file: {:?}", file);
            let mut device = Device::new_from_fd(file)?;
            device.grab(evdev_rs::GrabMode::Grab)?;
            Ok(device)
        })
        .collect::<io::Result<Vec<Device>>>()?;

    let output_devices = devices
        .iter()
        .map(|device| UInputDevice::create_from_device(device))
        .collect::<io::Result<Vec<UInputDevice>>>()?;

    let mut events_buffer = LinkedList::<(Instant, usize, InputEvent)>::new();

    // create buffer for epoll to fill
    let mut events = [epoll::Event::new(epoll::Events::empty(), 0); 4];
    loop {
        //processing queued events...
        while events_buffer
            .front()
            .map(|(earlier, _, _)| earlier.elapsed() > Duration::from_millis(ms_delay.into()))
            .unwrap_or(false)
        {
            let (earlier, idx, event) = events_buffer.pop_front().unwrap();
            let output_device = output_devices.get(idx).unwrap();
            let time_waited = earlier.elapsed();
            println!(
                "Actual amount of time waited: {}.{} ms",
                time_waited.as_millis(),
                time_waited.as_micros()
            );
            output_device.write_event(&event).unwrap();
        }
        //wait until new events need to be processed or new events arrive
        let wait_ms = events_buffer
            .front()
            .and_then(|(earlier, _, _)| {
                Duration::from_millis(u64::from(ms_delay) + 1u64).checked_sub(earlier.elapsed())
            })
            .and_then(|dur| dur.as_millis().try_into().ok())
            .unwrap_or(i32::max_value());
        println!("waiting... {:?}ms", wait_ms);
        //add new events to queue
        let num_events = epoll::wait(epoll_fd, wait_ms, &mut events)?;
        for event in &events[0..num_events] {
            let device_idx = event.data as usize;
            let device = devices.get(device_idx).unwrap();
            while device.has_event_pending() {
                //TODO: deal with EV_SYN::SYN_DROPPED
                let (_, event) = device.next_event(evdev_rs::ReadFlag::NORMAL)?;
                //println!("Got event: {:?} status", event);
                events_buffer.push_back((Instant::now(), device_idx, event));
            }
        }
    }
}
