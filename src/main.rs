use evdev_rs::{
    Device, InputEvent, UInputDevice,
};
use std::ffi::OsStr;
use std::fs::{read_dir, File};
use std::os::unix::{fs::FileTypeExt, io::AsRawFd};
//TODO - switch to mio
use epoll::ControlOptions;
use std::collections::LinkedList;
use std::thread::sleep;
use std::time::{Duration, Instant};

fn get_all_device_files() -> std::io::Result<Vec<File>> {
    let mut res = Vec::new();
    for entry in read_dir("/dev/input")? {
        let entry = entry?;
        // /dev/input files are character devices
        if !entry.file_type()?.is_char_device() {
            continue;
        }

        //TODO: consider using /dev/input/by-id
        // let location = std::fs::read_link(entry.path())?;
        // let real_location = entry.path().ancestors().nth(1).unwrap().join(location).canonicalize();
        // println!("opening: {:?}, real location: {:?}", entry.path(), real_location);
        // let file = File::open(entry.path())?;
        // res.push(file);

        // these files don't play nice with libevdev, not sure why
        let ignore = [OsStr::new("mouse0"), OsStr::new("mice")];
        match entry.path().file_name() {
            Some(file_name) => {
                if !ignore.contains(&file_name) {
                    println!("opening: {:?}", entry.path());
                    let file = File::open(entry.path())?;
                    res.push(file);
                }
            }
            None => {}
        }
    }
    Ok(res)
}
fn main() -> std::io::Result<()> {
    //println!("{:?}", std::env::args().collect::<Vec<_>>());
    let ms_delay: u32 = std::env::args().nth(1).map(|arg| arg.parse().unwrap()).unwrap_or(60);

    sleep(Duration::from_millis(250));

    println!("opening /dev/input files...");
    let device_files = get_all_device_files()?;
    println!("opened /dev/input files");

    println!("setting up epoll...");
    let epoll_fd = epoll::create(true)?;
    //add file descriptors to epoll
    for (file_idx, file) in device_files.iter().enumerate() {
        let device_fd = file.as_raw_fd();
        let epoll_event = epoll::Event::new(epoll::Events::EPOLLIN, file_idx as u64);
        epoll::ctl(
            epoll_fd,
            ControlOptions::EPOLL_CTL_ADD,
            device_fd,
            epoll_event,
        )?;
    }
    println!("epoll set up!");

    let devices = device_files
        .into_iter()
        .map(|file| {
            println!("setting up device for file: {:?}", file);
            let mut device = Device::new_from_fd(file)?;
            device.grab(evdev_rs::GrabMode::Grab)?;
            Ok(device)
        })
        .collect::<std::io::Result<Vec<Device>>>()?;

    let output_devices = devices
        .iter()
        .map(|device| UInputDevice::create_from_device(device))
        .collect::<std::io::Result<Vec<UInputDevice>>>()?;

    let mut events_buffer: LinkedList<(usize, Instant, InputEvent)> = LinkedList::new();

    // create buffer for epoll to fill
    let mut events = [epoll::Event::new(epoll::Events::empty(), 0); 4];
    for _idx in 0_usize.. {
        //processing queued events...
        while events_buffer
            .front()
            .map(|(_, earlier, _)| {
                Instant::now().duration_since(*earlier) > Duration::from_millis(ms_delay.into())
            })
            .unwrap_or(false)
        {
            //println!("processing events. Events remaining: {}", events_buffer.len());
            let (idx, _, event) = events_buffer.pop_front().unwrap();
            let output_device = output_devices.get(idx).unwrap();
            output_device.write_event(&event).unwrap();
        }
        //println!("waiting for the {}-th event...", idx);
        let num_events = epoll::wait(epoll_fd, (ms_delay / 5) as i32, &mut events)?;
        for event in &events[0..num_events] {
            let device_idx = event.data as usize;
            let device = devices.get(device_idx).unwrap();
            while device.has_event_pending() {
                let (_status, event) = device.next_event(evdev_rs::ReadFlag::NORMAL)?;
                //println!("Got event: {:?} status", event);
                events_buffer.push_back((device_idx, Instant::now(), event));
            }
        }
    }
    Ok(())
}
