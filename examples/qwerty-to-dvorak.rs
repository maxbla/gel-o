/// Make your QWERTY keyboard act like a DVORAK...
/// the wrong way
use evdev_rs::InputEvent;
use evdev_rs::enums::{EventCode, EV_KEY};
use gelo::{filter_map_events, GrabStatus};

fn qwerty_to_dvorak(event: InputEvent) -> InputEvent {
    match event.event_code {
        EventCode::EV_KEY(key) => {
            let key = match key {
                EV_KEY::KEY_Q => EV_KEY::KEY_APOSTROPHE,
                EV_KEY::KEY_W => EV_KEY::KEY_COMMA,
                EV_KEY::KEY_E => EV_KEY::KEY_DOT,
                EV_KEY::KEY_R => EV_KEY::KEY_P,
                EV_KEY::KEY_T => EV_KEY::KEY_Y,
                EV_KEY::KEY_Y => EV_KEY::KEY_F,
                EV_KEY::KEY_U => EV_KEY::KEY_G,
                EV_KEY::KEY_I => EV_KEY::KEY_C,
                EV_KEY::KEY_O => EV_KEY::KEY_R,
                EV_KEY::KEY_P => EV_KEY::KEY_L,
                EV_KEY::KEY_LEFTBRACE => EV_KEY::KEY_SLASH,
                EV_KEY::KEY_RIGHTBRACE => EV_KEY::KEY_EQUAL,

                EV_KEY::KEY_A => EV_KEY::KEY_A,
                EV_KEY::KEY_S => EV_KEY::KEY_O,
                EV_KEY::KEY_D => EV_KEY::KEY_E,
                EV_KEY::KEY_F => EV_KEY::KEY_U,
                EV_KEY::KEY_G => EV_KEY::KEY_I,
                EV_KEY::KEY_H => EV_KEY::KEY_D,
                EV_KEY::KEY_J => EV_KEY::KEY_H,
                EV_KEY::KEY_K => EV_KEY::KEY_T,
                EV_KEY::KEY_L => EV_KEY::KEY_N,
                EV_KEY::KEY_SEMICOLON => EV_KEY::KEY_S,
                EV_KEY::KEY_APOSTROPHE => EV_KEY::KEY_MINUS,

                EV_KEY::KEY_Z => EV_KEY::KEY_SEMICOLON,
                EV_KEY::KEY_X => EV_KEY::KEY_Q,
                EV_KEY::KEY_C => EV_KEY::KEY_J,
                EV_KEY::KEY_V => EV_KEY::KEY_K,
                EV_KEY::KEY_B => EV_KEY::KEY_X,
                EV_KEY::KEY_N => EV_KEY::KEY_B,
                EV_KEY::KEY_M => EV_KEY::KEY_M,
                EV_KEY::KEY_COMMA => EV_KEY::KEY_W,
                EV_KEY::KEY_DOT => EV_KEY::KEY_V,
                EV_KEY::KEY_SLASH => EV_KEY::KEY_Z,
                _ => key
            };
            InputEvent{event_code: EventCode::EV_KEY(key), ..event}
        },
        _ => event
    }
}

fn main() -> std::io::Result<()> {
    // wait for enter key to be released after starting
    // TODO: remove this hack
    std::thread::sleep(std::time::Duration::from_millis(500));

    let mut event_count = 0;

    filter_map_events(move |event| {
        let event = qwerty_to_dvorak(event);

        event_count += 1;
        // Ensure system doesn't become unusable by ungrabbing after many events
        if event_count >= 10000 {
            (None, GrabStatus::Stop)
        } else {
            (Some(event), GrabStatus::Continue)
        }
    })?;
    Ok(())
}