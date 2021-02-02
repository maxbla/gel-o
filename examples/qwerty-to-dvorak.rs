//! Make your QWERTY keyboard act like a DVORAK...
//! the wrong way
use evdev_rs::{
    InputEvent,
    enums::{EventCode, EV_KEY}
};
use EV_KEY::*;
use gelo::EventsListener;
use std::thread::sleep;
use std::time::Duration;

fn qwerty_to_dvorak(event: InputEvent) -> InputEvent {
    match event.event_code {
        EventCode::EV_KEY(key) => {
            let key = match key {
                KEY_Q => KEY_APOSTROPHE,
                KEY_W => KEY_COMMA,
                KEY_E => KEY_DOT,
                KEY_R => KEY_P,
                KEY_T => KEY_Y,
                KEY_Y => KEY_F,
                KEY_U => KEY_G,
                KEY_I => KEY_C,
                KEY_O => KEY_R,
                KEY_P => KEY_L,
                KEY_LEFTBRACE => KEY_SLASH,
                KEY_RIGHTBRACE => KEY_EQUAL,

                KEY_A => KEY_A,
                KEY_S => KEY_O,
                KEY_D => KEY_E,
                KEY_F => KEY_U,
                KEY_G => KEY_I,
                KEY_H => KEY_D,
                KEY_J => KEY_H,
                KEY_K => KEY_T,
                KEY_L => KEY_N,
                KEY_SEMICOLON => KEY_S,
                KEY_APOSTROPHE => KEY_MINUS,

                KEY_Z => KEY_SEMICOLON,
                KEY_X => KEY_Q,
                KEY_C => KEY_J,
                KEY_V => KEY_K,
                KEY_B => KEY_X,
                KEY_N => KEY_B,
                KEY_M => KEY_M,
                KEY_COMMA => KEY_W,
                KEY_DOT => KEY_V,
                KEY_SLASH => KEY_Z,
                _ => key,
            };
            InputEvent {
                event_code: EventCode::EV_KEY(key),
                ..event
            }
        }
        _ => event,
    }
}

fn main() -> std::io::Result<()> {
    // wait for enter key to be released after starting
    // TODO: remove this hack
    sleep(Duration::from_millis(500));

    let mut listener = EventsListener::new(true)?;
    for (event, device) in listener.iter().take(10) {
        println!("[QWERTY Key] event: {:?}", event);
        let event = qwerty_to_dvorak(event);
        println!("[DVORAK Key] event: {:?}", event);
        #[cfg(feature = "arc")]
        device.lock().unwrap().write_event(&event)?;
        #[cfg(not(feature = "arc"))]
        device.write_event(&event)?;
    }

    Ok(())
}
