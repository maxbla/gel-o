pub use evdev_rs;

use epoll::ControlOptions::{EPOLL_CTL_ADD, EPOLL_CTL_DEL};
use evdev_rs::{Device, InputEvent, ReadFlag, UInputDevice};
use inotify::{EventMask, Inotify, WatchMask};
use std::os::unix::{
    ffi::OsStrExt,
    fs::FileTypeExt,
    io::{AsRawFd, FromRawFd, RawFd},
};
#[cfg(feature = "arc")]
use std::sync::Mutex;
use std::{
    collections::{BTreeMap, HashMap},
    convert::TryInto,
    ffi::{CString, OsStr, OsString},
    fs::{read_dir, File},
    io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

static DEV_PATH: &str = "/dev/input";
const EPOLLIN: epoll::Events = epoll::Events::EPOLLIN;

#[cfg(feature = "arc")]
pub type Ptr<T> = std::sync::Arc<Mutex<T>>;
#[cfg(not(feature = "arc"))]
pub type Ptr<T> = std::rc::Rc<T>;

/// path to a file and the file itself
/// generic so you can use file-backed types which are not `std::io::File`s,
/// for example evdev::Device
pub type PathFile<T> = (PathBuf, T);

pub type DeviceFilter = fn(&Device) -> bool;

/// A handle to all resources needed to listen for and simulate input events
///
/// Creating multiple `EventsListener`s is fraught, as the order of event
/// delivery to `EventsListener`s is unspecified
pub struct EventsListener {
    epoll_fd: RawFd,
    epoll_buffer: Option<[epoll::Event; 1]>,
    /// monotonically increasing index for inserting into hashmap
    idx: u64,
    /// corresponding device path, input and output devices
    devices: HashMap<u64, (PathFile<Device>, PathFile<Ptr<UInputDevice>>)>,
    grab: bool,
    inotify: Inotify,
    inotify_buffer: Vec<u8>,
    filter: DeviceFilter,
}

impl Drop for EventsListener {
    fn drop(&mut self) {
        if self.grab {
            for ((_path, device), (_, _)) in self.devices.values_mut() {
                //ungrab devices, ignore errors
                device.grab(evdev_rs::GrabMode::Ungrab).ok();
            }
        }
        epoll::close(self.epoll_fd).ok();
    }
}

impl EventsListener {
    /// Gets a new events listener.
    ///
    /// Grab determines whether the events are automatically delivered further
    /// up the stack. If grabbing, you must simulate any event you recieve
    /// gor the OS to see it.
    pub fn new(grab: bool) -> io::Result<EventsListener> {
        let filter = |_: &Device| true;
        Self::new_with_filter(grab, filter)
    }

    /// Like new, only allows a filter, so some devices are not grabbed.
    ///
    /// filter must return the same value when called multiple times on the
    /// same device
    ///
    /// This method is immensely usefuly for testing, as it allows capturing
    /// only mouse input (in the event of a bug, you can press ctrl+c or even
    /// switch to the linux virtual terminal with ctrl+alt+F6 then use kill/htop)
    pub fn new_with_filter(grab: bool, filter: DeviceFilter) -> io::Result<EventsListener> {
        let (epoll_fd, devices) = setup_devices(filter)?;
        let mut devices: HashMap<u64, _> = devices
            .into_iter()
            .map(|(idx, ((path, dev), (ui_path, ui_dev)))| {
                #[cfg(feature = "arc")]
                return (idx, ((path, dev), (ui_path, Ptr::new(Mutex::new(ui_dev)))));
                #[cfg(not(feature = "arc"))]
                (idx, ((path, dev), (ui_path, Ptr::new(ui_dev))))
            })
            .collect();
        let mut inotify = Inotify::init()?;
        inotify.add_watch(DEV_PATH, WatchMask::CREATE | WatchMask::DELETE)?;

        if grab {
            for ((_, device), _) in devices.values_mut() {
                device.grab(evdev_rs::GrabMode::Grab)?
            }
        }
        let inotify_buffer = vec![0_u8; 4096];

        Ok(EventsListener {
            epoll_fd,
            epoll_buffer: None,
            idx: devices.len().try_into().unwrap(),
            devices,
            grab,
            inotify,
            inotify_buffer,
            filter,
        })
    }

    pub fn iter(&mut self) -> InputIter {
        InputIter {
            listener: self,
            last_device_idx: None,
            syncing: false,
        }
    }

    /// Simulate an event on a device
    ///
    /// Panics if you give it an index that does not correspond to a device
    /// Usual EV_SYN caveat applies - events not processed until EV_SYN/SYN_REPORT is sent
    /// Prefer iter()
    pub fn simulate(&self, event: InputEvent, device_idx: u64) -> io::Result<()> {
        let output_device = self
            .devices
            .get(&device_idx)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "index out of range"))?
            .1
             .1
            .clone();
        #[cfg(feature = "arc")]
        output_device
            .try_lock()
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "Could not aquire lock"))?
            .write_event(&event)?;
        #[cfg(not(feature = "arc"))]
        output_device.write_event(&event)?;
        Ok(())
    }
}

/// An iterator over evdev InputEvents
pub struct InputIter<'a> {
    listener: &'a mut EventsListener,
    last_device_idx: Option<u64>,
    syncing: bool,
}

impl<'a> InputIter<'a> {
    /// Returns true if a new event is queued up (calling next() would not block)
    pub fn ready(&mut self) -> io::Result<bool> {
        match self.listener.epoll_buffer {
            Some(_) => Ok(true),
            None => {
                let mut epoll_buffer = [epoll::Event::new(epoll::Events::empty(), 0); 1];
                let num_events = epoll::wait(self.listener.epoll_fd, 0, &mut epoll_buffer)?;
                if num_events > 0 {
                    self.listener.epoll_buffer = Some(epoll_buffer);
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }

    /// gets next event from a device, taking self.syncing into account.
    /// non-blocking.
    /// Return value of Ok(None) indicates no event was available
    fn device_next(
        &mut self,
        device_idx: u64,
    ) -> io::Result<Option<(&Device, InputEvent, Ptr<UInputDevice>)>> {
        let listener = &mut self.listener;
        if let Some(((_path, device), (_ui_path, uinput_device))) =
            listener.devices.get(&device_idx)
        {
            if device.has_event_pending() {
                let (_read_status, event) = if self.syncing {
                    match device.next_event(ReadFlag::SYNC) {
                        Ok(event) => event,
                        Err(code) if code.raw_os_error() == Some(libc::EAGAIN) => {
                            self.syncing = false;
                            device.next_event(ReadFlag::NORMAL)?
                        }
                        Err(err) => return Err(err),
                    }
                } else {
                    device.next_event(ReadFlag::NORMAL)?
                };
                self.last_device_idx = Some(device_idx);
                return Ok(Some((device, event, uinput_device.clone())));
            }
        };
        Ok(None)
    }

    /// Blocks until either an Input event occurs or timeout time passes
    ///
    /// A timeout of None indicates infinite timeout.
    /// Panics if timeout is longer than `i32::MAX` milliseconds
    /// returning Ok(None) indicates the timeout expired
    ///
    pub fn next_timeout(
        &mut self,
        timeout: Option<Duration>,
    ) -> io::Result<Option<(InputEvent, Ptr<UInputDevice>)>> {
        // determine if previous device has more events
        // necessary because evdev batch reads events off the fd
        if let Some(last_idx) = self.last_device_idx {
            if let Ok(Some(event)) = self.device_next(last_idx) {
                return Ok(Some((event.1, event.2)));
            }
        };

        let end_instant = timeout.map(|timeout| Instant::now() + timeout);
        while end_instant
            .map(|end_instant| Instant::now() < end_instant)
            .unwrap_or(true)
        {
            // eagerly update map of devcie files
            // previously, this was done through epoll,
            // but that caused bugs, as if a device was un plugged, then plugged back it
            // it might have been ignored by device de-dupe logic
            for event in self
                .listener
                .inotify
                .read_events(&mut self.listener.inotify_buffer)?
            {
                if event.mask.contains(EventMask::CREATE | EventMask::DELETE) {
                    panic!("Both created and deleted. Logic does not properly handle this case");
                } else if event.mask.contains(EventMask::CREATE) {
                    let is_uidevice = self
                        .listener
                        .devices
                        .values()
                        .any(|(_, (ui_path, _))| ui_path.file_name() == event.name);
                    if !is_uidevice {
                        let maybe_device = get_device_from_inotify_event(event);
                        let (path, mut device) = match maybe_device {
                            None => continue, //file cannot be used to initalize a Device. Skip.
                            Some(device) => device?,
                        };
                        let filter: DeviceFilter = self.listener.filter;
                        if !filter(&device) {
                            continue;
                        }
                        if self.listener.grab {
                            device.grab(evdev_rs::GrabMode::Grab)?;
                        }
                        let out_device = UInputDevice::create_from_device(&device)?;
                        let ui_path = PathBuf::from(out_device.devnode().unwrap_or(""));
                        let fd = device.file().as_raw_fd();
                        let idx = &mut self.listener.idx;
                        let event = epoll::Event::new(EPOLLIN, *idx);
                        #[cfg(not(feature = "arc"))]
                        self.listener
                            .devices
                            .insert(*idx, ((path, device), (ui_path, Ptr::new(out_device))));
                        #[cfg(feature = "arc")]
                        self.listener.devices.insert(
                            *idx,
                            ((path, device), (ui_path, Ptr::new(Mutex::new(out_device)))),
                        );
                        *idx += 1;
                        epoll::ctl(self.listener.epoll_fd, EPOLL_CTL_ADD, fd, event)?;
                    }
                } else if event.mask.contains(EventMask::DELETE) {
                    if let Some(((_, device), _)) = self
                        .listener
                        .devices
                        .values()
                        .find(|((path, _), ..)| path.file_name() == event.name)
                    {
                        epoll::ctl(
                            self.listener.epoll_fd,
                            EPOLL_CTL_DEL,
                            device.file().as_raw_fd(),
                            epoll::Event::new(epoll::Events::empty(), 0),
                        )?;
                    } else {
                        // simply ignore. It would be nice to run a sanity check here,
                        // bit that's not possible
                        // panic!("Device that doesn't exist was unplugged")
                    }
                    // remove all devices with same path as deleted file
                    self.listener
                        .devices
                        .retain(|_idx, ((path, _), ..)| path.file_name() != event.name);
                } else {
                    panic!("inotify is listening for events other than file creation");
                }
            }

            let event = match self.listener.epoll_buffer {
                Some(epoll_buffer) => {
                    let event = epoll_buffer[0];
                    self.listener.epoll_buffer = None;
                    event
                }
                None => {
                    let mut epoll_buffer = [epoll::Event::new(epoll::Events::empty(), 0); 1];
                    let timeout_millis = if let Some(end_instant) = end_instant {
                        let now = Instant::now();
                        if now < end_instant {
                            end_instant
                                .duration_since(now)
                                .as_millis()
                                .try_into()
                                .map_err(|_| {
                                    io::Error::new(io::ErrorKind::InvalidInput, "timeout too long")
                                })?
                        } else {
                            0
                        }
                    } else {
                        -1
                    };
                    let num_events =
                        epoll::wait(self.listener.epoll_fd, timeout_millis, &mut epoll_buffer)?;
                    match num_events {
                        0 => return Ok(None),
                        1 => epoll_buffer[0],
                        _ => panic!("epoll_wait didn't work as expected"),
                    }
                }
            };

            let device_idx = event.data;
            return self
                .device_next(device_idx)
                .map(|event| event.map(|inner| (inner.1, inner.2)));
        }
        Ok(None)
    }
}

impl<'a> std::iter::Iterator for InputIter<'a> {
    type Item = (InputEvent, Ptr<UInputDevice>);

    fn size_hint(&self) -> (usize, Option<usize>) {
        (std::usize::MAX, None)
    }

    /// Get the next event from all devices included in the DeviceListener
    /// Returning None indicates an io Error. To get the full io error, use next_timeout
    fn next(&mut self) -> Option<Self::Item> {
        self.next_timeout(None).ok().flatten()
    }
}

/// Whether to continue grabbing events or to stop
/// Used in `filter_map_events` (and others)
#[deprecated(since = "0.1.0", note = "This is only used with deprecated methods")]
#[derive(Debug, Eq, PartialEq, Hash)]
pub enum GrabStatus {
    /// Stop grabbing
    Continue,
    /// ungrab events
    Stop,
}

fn get_device_paths<T: AsRef<Path>>(path: T) -> io::Result<Vec<PathBuf>> {
    let mut res = Vec::new();
    for entry in read_dir(path)? {
        let entry = entry?;
        // /dev/input files are character devices
        if !entry.file_type()?.is_char_device() {
            continue;
        }

        let path = entry.path();
        let file_name_bytes = match path.file_name() {
            Some(file_name) => file_name.as_bytes(),
            None => continue, // file_name was "..", should be impossible
        };
        // skip filenames matching "mouse.* or mice".
        // these files don't play nice with libevdev, not sure why
        // see: https://askubuntu.com/questions/1043832/difference-between-dev-input-mouse0-and-dev-input-mice
        if file_name_bytes == OsStr::new("mice").as_bytes()
            || file_name_bytes
                .get(0..=1)
                .map(|s| s == OsStr::new("js").as_bytes())
                .unwrap_or(false)
            || file_name_bytes
                .get(0..=4)
                .map(|s| s == OsStr::new("mouse").as_bytes())
                .unwrap_or(false)
        {
            continue;
        }
        res.push(path);
    }
    Ok(res)
}

fn get_device_from_inotify_event(
    event: inotify::Event<&OsStr>,
) -> Option<io::Result<(PathBuf, Device)>> {
    if event.mask != EventMask::CREATE {
        return None;
    }
    // skip filenames matching "mouse.* or mice".
    // these files don't play nice with libevdev, not sure why
    // see: https://askubuntu.com/questions/1043832/difference-between-dev-input-mouse0-and-dev-input-mice
    let new_dev_filename = event.name?;
    if new_dev_filename == OsStr::new("mice")
        || new_dev_filename
            .as_bytes()
            .get(0..=1)
            .map(|s| s == OsStr::new("js").as_bytes())
            .unwrap_or(false)
        || new_dev_filename
            .as_bytes()
            .get(0..=4)
            .map(|s| s == OsStr::new("mouse").as_bytes())
            .unwrap_or(false)
    {
        return None;
    }

    let mut device_path = OsString::from(DEV_PATH);
    device_path.push(OsString::from("/"));
    device_path.push(new_dev_filename);
    let device_path: PathBuf = device_path.into();
    // new plug events
    let file = match open_file_nonblock(&device_path) {
        Ok(file) => file,
        Err(err) => return Some(Err(err)),
    };
    Some(Device::new_from_file(file).map(|device| (device_path, device)))
}

/// Open an fd with O_RDONLY | O_NONBLOCK
fn open_file_nonblock<P: AsRef<OsStr>>(path: P) -> io::Result<File> {
    let path = CString::new(path.as_ref().as_bytes())
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let fd = match unsafe { libc::open(path.into_raw(), libc::O_RDONLY | libc::O_NONBLOCK) } {
        -1 => return Err(io::Error::last_os_error()),
        res => res,
    };
    let mut buf = [0u8; 128];
    let mut res = 0;
    while res >= 0 {
        res = unsafe { libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, 128) };
    }
    assert!(io::Error::last_os_error().kind() == io::ErrorKind::WouldBlock);
    Ok(unsafe { File::from_raw_fd(fd) })
}

/// Returns tuple of epoll_fd and a hashmap containing a device and a
/// UInputDevice that any events read from the device can be simulated on.
/// The epoll_fd is level-triggered on the device file.
fn setup_devices(
    include: DeviceFilter,
) -> io::Result<(
    RawFd,
    HashMap<u64, (PathFile<Device>, PathFile<UInputDevice>)>,
)> {
    let device_paths = get_device_paths(DEV_PATH)?;
    let epoll_fd = epoll::create(true)?;
    let mut ret = HashMap::with_capacity(device_paths.len());
    for (idx, path) in device_paths.into_iter().enumerate() {
        let file = open_file_nonblock(path.clone())?;
        let device = Device::new_from_file(file)?;
        if !include(&device) {
            continue;
        }
        let idx = idx.try_into().unwrap();
        let epoll_event = epoll::Event::new(EPOLLIN, idx);
        epoll::ctl(
            epoll_fd,
            EPOLL_CTL_ADD,
            device.file().as_raw_fd(),
            epoll_event,
        )?;
        let output_device = UInputDevice::create_from_device(&device)?;
        let devnode = output_device.devnode().unwrap();
        let uidev_path = PathBuf::from(devnode);
        ret.insert(idx, ((path, device), (uidev_path, output_device)));
    }
    Ok((epoll_fd, ret))
}

#[deprecated(
    since = "0.1.0",
    note = "Use EventListener.iter().filter_map with your own closure that
    schedules delayed events. See examples/delay.rs"
)]
// TODO: return the never type, `!` when it is stabalized
/// Like `filter_map_events_with_delay`, only never returns
pub fn filter_map_events_with_delay_noreturn<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> (Instant, Option<InputEvent>),
{
    #[allow(deprecated)]
    filter_map_events_with_delay(|input_event| {
        let (instant, output_event) = func(input_event);
        (instant, output_event, GrabStatus::Continue)
    })
}

#[deprecated(
    since = "0.1.0",
    note = "Use EventListener.iter().take_while().filter_map with your own
    closure that schedules delayed events. See examples/delay.rs"
)]
#[allow(deprecated)]
/// Similar to Iterator's filter_map, only this blocks waiting for input
///
/// Filter and transform events, additionally specifying an Instant at which
/// to simulate the transformed event. To stop grabbing, return
/// `GrabStatus::Stop`
pub fn filter_map_events_with_delay<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> (Instant, Option<InputEvent>, GrabStatus),
{
    let mut listener = EventsListener::new(true)?;
    let mut event_iter = listener.iter();
    let mut events_buffer = BTreeMap::<(Instant, u64), (InputEvent, Ptr<UInputDevice>)>::new();
    // Generation acts as a key for events_buffer, so that if two events occur
    // at the same Instant, they can co-exist in the BTreeMap
    let mut generation: u64 = 0;
    'outer: loop {
        let process_later = events_buffer.split_off(&(Instant::now(), 0));
        let process_now = events_buffer;
        events_buffer = process_later;

        for (event, device) in process_now.values() {
            #[cfg(feature = "arc")]
            device.lock().unwrap().write_event(event)?;
            #[cfg(not(feature = "arc"))]
            device.write_event(event)?;
        }

        while events_buffer
            .keys()
            .next()
            .map(|(instant, _)| &Instant::now() < instant)
            .unwrap_or(true)
        {
            let event = match events_buffer.keys().next() {
                None => event_iter.next_timeout(None)?,
                Some((send_instant, _)) => {
                    let now = Instant::now();
                    if &now < send_instant {
                        event_iter.next_timeout(Some(send_instant.duration_since(now)))?
                    } else {
                        event_iter.next_timeout(Some(Duration::from_millis(0)))?
                    }
                }
            };
            if let Some((event, device)) = event {
                let (instant, event, grab) = func(event);
                if grab == GrabStatus::Stop {
                    break 'outer Ok(());
                }
                if let Some(event) = event {
                    let entry = events_buffer.insert((instant, generation), (event, device));
                    assert!(
                        entry.is_none(),
                        "Two events occured at the same instant with same generation"
                    );
                    generation = generation.wrapping_add(1);
                }
            }
        }
    }
}

// TODO: return the never type, `!` when it is stabalized
/// Like `filter_map_events`, only never returns
#[deprecated(since = "0.1.0", note = "Use EventListener.iter().filter_map instead")]
pub fn filter_map_events_noreturn<F>(func: F) -> io::Result<()>
where
    F: Fn(InputEvent) -> Option<InputEvent>,
{
    #[allow(deprecated)]
    filter_map_events(|input_event| {
        let output_event = func(input_event);
        (output_event, GrabStatus::Continue)
    })
}

#[deprecated(
    since = "0.1.0",
    note = "Use EventListener.iter().take_while().filter_map instead"
)]
#[allow(deprecated)]
/// Similar to Iterator's filter_map, only this blocks waiting for input
///
/// Filter and transform events. To stop grabbing, return `GrabStatus::Stop`
pub fn filter_map_events<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> (Option<InputEvent>, GrabStatus),
{
    let mut listener = EventsListener::new(true)?;
    listener
        .iter()
        .map(|(event, device)| {
            let (new_event, status) = func(event);
            (new_event, device, status)
        })
        .take_while(|(_, _, status)| status != &GrabStatus::Stop)
        .filter_map(|(event, device, _)| event.map(|event| (event, device)))
        .try_for_each(|(event, device)| {
            #[cfg(feature = "arc")]
            return device.lock().unwrap().write_event(&event);
            #[cfg(not(feature = "arc"))]
            device.write_event(&event)
        })
}

#[deprecated(since = "0.1.0", note = "Use EventListener.iter().for_each instead")]
#[allow(deprecated)]
/// Similar to Iterator's for_each, only this blocks waiting for input
///
/// Calls a closure on each event. To stop grabbing, return `GrabStatus::Stop`
pub fn for_each_event<F>(mut func: F) -> io::Result<()>
where
    F: FnMut(InputEvent) -> GrabStatus,
{
    let mut listener = EventsListener::new(false)?;
    for (event, _) in listener.iter() {
        if func(event) == GrabStatus::Stop {
            break;
        }
    }
    Ok(())
}
