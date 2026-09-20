//! The lobby (docs/online-coop-prd.md §4.10): hosting and joining a room
//! as something a player does on screen, so `--host`, `--join CODE` and
//! `--rooms` stop being the only way in.
//!
//! It is a mode of its own beside Play and Build - `mode::Driver::Lobby`,
//! entered by the bar's `ONLINE` button - rather than a face of
//! `Driver::Online`, because it exists before any room does: pressing
//! `ONLINE` opens it with no socket, no seat and nothing to draw a round
//! from. Once the room says the round has begun, `Session` hands over to
//! `Driver::Online` and the replica is what is on screen; the one amber
//! status line that used to carry the whole lobby is back to being the
//! round's own (`net::round::OnlineRound::status`).
//!
//! Headless, like `hud.rs`: this half is the state machine, the geometry
//! (`button_rect` is the one table the drawing and every hit test read)
//! and the view a painter needs; `render::lobby` is the painting. The
//! screen never opens a socket either - `HOST` and `JOIN` come back as a
//! [`LobbyAction`] and `app.rs` builds the transport, so a button lands on
//! exactly the path the command line takes.

use crate::level::Mission;
use crate::map::SHIPPED_MAPS;
use crate::math::{Rectangle, Vec2};
use crate::net::client::Phase;
use crate::net::rooms::{self, CODE_ALPHABET, ROOM_LETTERS, RoomCode, RoomsHost};
use crate::net::round::OnlineRound;
use crate::net::transport::Transport;
use crate::net::wire::{RosterSeat, RoundOutcome};
use crate::qr::Qr;
use crate::tank::TankKind;
use crate::Rect;

/// The panel, centred in the field. Fixed rather than fitted to the
/// window so no button moves under a finger, and small enough for the
/// smallest field the game ships (`maps/crossplay/`, 24 x 12 cells =
/// 768 x 384 px).
pub const LOBBY_W: f32 = 704.0;
pub const LOBBY_H: f32 = 336.0;
/// The gutter between the panel's edge and anything in it.
pub const LOBBY_MARGIN: f32 = 20.0;

/// The action buttons along the panel's bottom, the leave dialog's height
/// so a finger has the same target it has everywhere else.
pub const LOBBY_BUTTON_W: f32 = 140.0;
pub const LOBBY_BUTTON_H: f32 = 48.0;
/// The two wide buttons of the opening face (`HOST A ROOM`, `JOIN`).
pub const LOBBY_WIDE_W: f32 = 200.0;

/// The smallest a button in this screen may be. The phones have no
/// keyboard at all (`KEYBOARD_AVAILABLE`), so every one of them is a
/// touch target; `lobby_tests` holds the whole screen to it.
pub const LOBBY_TOUCH_MIN: f32 = 44.0;

/// Seat rows the panel shows at once. A longer roster is a `+N MORE`
/// line under them - `MAX_SEATS` is eight and four rows is what fits
/// beside the QR on the smallest field.
pub const LOBBY_SEAT_ROWS: usize = 4;
pub const LOBBY_SEAT_H: f32 = 48.0;
pub const LOBBY_KICK_W: f32 = 72.0;

/// The square the QR is drawn in, top-right of the panel: 148 px holds a
/// version 3 code (37 modules with its quiet zone) at four pixels a
/// module, which is every link this game makes.
pub const LOBBY_QR_BOX: f32 = 148.0;

/// The code entry's five boxes.
pub const LOBBY_CODE_BOX: f32 = 64.0;
pub const LOBBY_CODE_GAP: f32 = 12.0;

/// The on-screen alphabet: twenty keys in two rows of ten.
pub const LOBBY_KEY_COLS: usize = 10;
pub const LOBBY_KEY_W: f32 = 58.0;
pub const LOBBY_KEY_H: f32 = 48.0;
pub const LOBBY_KEY_GAP: f32 = 8.0;

/// The missions a host can pick, in the order the stepper walks them.
pub const MISSIONS: [Mission; 3] = [Mission::Protect, Mission::Hunt, Mission::Destroy];

/// Which face of the lobby is up. Derived from whether a room has been
/// opened and how far along it is, never stored, so the screen can never
/// disagree with the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// No room yet: pick a map and a mission, then host or join.
    Start,
    /// Typing a room code.
    Code,
    /// Dialling, or waiting for the room to answer.
    Waiting,
    /// In the room: the code, the QR and the seats - before the round,
    /// and again after it, when the same face carries how it went and
    /// the host's action reads `REMATCH`.
    Room,
    /// The socket is gone, with why.
    Closed,
}

/// Every button the screen can show. One value is one rect
/// ([`button_rect`]) and one meaning, so a tap, a key and a test all land
/// in the same place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    /// Open a room with the chosen map and mission.
    Host,
    /// Go to the code entry, and from there take a seat.
    Join,
    /// Back one step: out of the code entry, or out of the lobby.
    Back,
    MapPrev,
    MapNext,
    MissionPrev,
    MissionNext,
    /// One key of the on-screen alphabet, indexing `CODE_ALPHABET`.
    Key(u8),
    /// Rub out the last character typed.
    Del,
    /// Take a seat in the room the entry spells.
    Confirm,
    /// Say this seat is ready.
    Ready,
    /// Start the round, or play it again once one has ended (the
    /// host's).
    Start,
    /// Give the seat up and come back to the local round.
    Leave,
    /// Put the seat on row `n` out of the room (the host's).
    Kick(u8),
}

/// What the screen asks its caller to do. Everything that needs a socket
/// or the session's own state leaves through here rather than being done
/// in the screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LobbyAction {
    None,
    /// Open a room: `map` is a `SHIPPED_MAPS` name.
    Host { map: String, mission: Mission },
    /// Take a seat in the room this code names, canonical and checked.
    Join { code: String },
    Ready,
    Start,
    Kick { seat: u8 },
    /// Close the lobby: hang up if there is a room, and come back to the
    /// local round exactly where it stood.
    Leave,
}

/// How far along the room is, as the screen needs it. One step coarser
/// than `net::client::Phase`: the screen does not care whether the socket
/// is still dialling or the create is in flight.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RoomPhase {
    Dialling,
    InRoom,
    Playing,
    Closed,
}

/// The room as the screen reads it: a plain snapshot taken once a frame,
/// so the screen's own tests need no socket.
#[derive(Clone, Debug, PartialEq)]
pub struct RoomView {
    pub phase: RoomPhase,
    pub code: Option<String>,
    pub seat: Option<u8>,
    pub host_seat: u8,
    pub is_host: bool,
    /// Pressing `START` now would be taken (the room server's own three
    /// conditions - see `OnlineRound::can_start`).
    pub can_start: bool,
    /// This seat has said it is ready.
    pub ready: bool,
    pub seats: Vec<RosterSeat>,
    /// The last thing the room refused, or why the socket went.
    pub note: Option<String>,
    /// How the round the room just finished went. While it is set the
    /// room face reads as an end screen: the outcome under the seats and
    /// `REMATCH` where `START` was.
    pub ended: Option<RoundOutcome>,
}

impl RoomView {
    /// The room a live seat is in.
    pub fn of<T: Transport>(round: &OnlineRound<T>) -> RoomView {
        let seat = round.seat();
        let phase = match round.phase() {
            Phase::Connecting | Phase::Greeting => RoomPhase::Dialling,
            Phase::Lobby => RoomPhase::InRoom,
            Phase::Playing => RoomPhase::Playing,
            Phase::Closed(_) => RoomPhase::Closed,
        };
        RoomView {
            phase,
            code: round.code().map(str::to_string),
            seat,
            host_seat: round.host_seat(),
            is_host: round.is_host(),
            can_start: round.can_start(),
            ready: round.roster().iter().any(|s| Some(s.seat) == seat && s.ready),
            seats: round.roster().to_vec(),
            note: round.note().map(str::to_string),
            ended: round.ended(),
        }
    }
}

/// One seat, as a row on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeatRow {
    /// The seat number, which is the tank's owner slot.
    pub seat: u8,
    /// `P1`..`P8`, the label the tank wears on the field.
    pub slot: String,
    pub nick: String,
    /// The chassis name, or `-` before the room has rolled one.
    pub chassis: &'static str,
    /// `HOST`, `READY`, `WAITING` or `AWAY`.
    pub state: &'static str,
    pub ready: bool,
    pub you: bool,
    pub host: bool,
}

/// One button as the painter draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonView {
    pub button: Button,
    pub label: String,
    /// A press would do something; a disabled button is drawn dim and
    /// ignored by the hit test.
    pub enabled: bool,
    /// The action colour (the builder's amber) rather than plain white.
    pub accent: bool,
}

/// Everything `render::lobby` draws, as plain values - the `HudModel`
/// pattern. Built once a frame by [`Lobby::view`].
#[derive(Clone, Debug, PartialEq)]
pub struct LobbyView {
    pub stage: Stage,
    pub title: String,
    /// The line under the title: what to do, or what went wrong.
    pub sub: String,
    pub code: Option<String>,
    pub join_url: Option<String>,
    pub qr: Option<Qr>,
    pub map: String,
    pub mission: Mission,
    /// The characters typed so far, in the code entry.
    pub entry: String,
    pub seats: Vec<SeatRow>,
    /// Seats past the rows there is room for.
    pub more: usize,
    pub buttons: Vec<ButtonView>,
}

/// The lobby's own state: what has been typed and picked. Everything
/// about the room itself is read off the client each frame.
pub struct Lobby {
    rooms: RoomsHost,
    /// The code entry is open (`JOIN` was pressed).
    entry_open: bool,
    entry: String,
    map: usize,
    mission: usize,
    /// Why the last code was refused, under the boxes.
    entry_note: Option<String>,
    /// The link's QR, kept with the link it was made from so it is
    /// encoded once rather than every frame.
    qr: Option<(String, Qr)>,
}

impl Lobby {
    /// A fresh lobby against `rooms` - the host `--rooms` or
    /// `BONGBONG_ROOMS` named, which is also what the invite link
    /// carries.
    pub fn new(rooms: RoomsHost) -> Lobby {
        Lobby { rooms, entry_open: false, entry: String::new(), map: 0, mission: 0, entry_note: None, qr: None }
    }

    /// Which face is up.
    pub fn stage(&self, room: Option<&RoomView>) -> Stage {
        match room {
            None if self.entry_open => Stage::Code,
            None => Stage::Start,
            Some(room) => match room.phase {
                RoomPhase::Dialling => Stage::Waiting,
                RoomPhase::InRoom | RoomPhase::Playing => Stage::Room,
                RoomPhase::Closed => Stage::Closed,
            },
        }
    }

    /// The shipped map this lobby would host.
    pub fn map(&self) -> &'static str {
        SHIPPED_MAPS[self.map % SHIPPED_MAPS.len()].0
    }

    pub fn mission(&self) -> Mission {
        MISSIONS[self.mission % MISSIONS.len()]
    }

    /// What has been typed into the code entry.
    pub fn entry(&self) -> &str {
        &self.entry
    }

    /// The buttons this face shows, in the order they are laid out. The
    /// hit test walks exactly this list, so a button that is not drawn
    /// cannot be pressed.
    pub fn buttons(&self, room: Option<&RoomView>) -> Vec<Button> {
        match self.stage(room) {
            Stage::Start => vec![Button::MapPrev, Button::MapNext, Button::MissionPrev, Button::MissionNext, Button::Host, Button::Join, Button::Back],
            Stage::Code => {
                let mut buttons: Vec<Button> = (0..CODE_ALPHABET.len() as u8).map(Button::Key).collect();
                buttons.extend([Button::Del, Button::Confirm, Button::Back]);
                buttons
            }
            Stage::Waiting | Stage::Closed => vec![Button::Leave],
            Stage::Room => {
                let room = room.expect("the room face has a room");
                let mut buttons = vec![Button::Ready, Button::Leave];
                if room.is_host {
                    buttons.push(Button::Start);
                    for (row, seat) in room.seats.iter().take(LOBBY_SEAT_ROWS).enumerate() {
                        if Some(seat.seat) != room.seat {
                            buttons.push(Button::Kick(row as u8));
                        }
                    }
                }
                buttons
            }
        }
    }

    /// The button under `point` (field space), if one is there and live.
    pub fn hit(&self, field: Rect, point: Vec2, room: Option<&RoomView>) -> Option<Button> {
        let enabled = |b: Button| self.enabled(b, room);
        self.buttons(room).into_iter().find(|&b| enabled(b) && button_rect(field, b).contains(point))
    }

    /// Whether pressing `button` would do anything.
    fn enabled(&self, button: Button, room: Option<&RoomView>) -> bool {
        match button {
            Button::Confirm => RoomCode::parse(&self.entry).is_ok(),
            Button::Start => room.is_some_and(|r| r.can_start),
            Button::Del => !self.entry.is_empty(),
            Button::Ready => room.is_some_and(|r| !r.ready),
            _ => true,
        }
    }

    /// One frame of the screen: the keys, then the pointer. `room` is the
    /// live room, absent until one has been opened.
    pub fn update(&mut self, input: &LobbyInput, field: Rect, room: Option<&RoomView>) -> LobbyAction {
        self.refresh_qr(room);
        let stage = self.stage(room);
        if let Some(action) = self.keys(input, stage, room) {
            return action;
        }
        let Some(point) = input.pointer.filter(|_| input.pressed) else { return LobbyAction::None };
        match self.hit(field, point, room) {
            Some(button) => self.press(button, room),
            None => LobbyAction::None,
        }
    }

    /// The keyboard, where there is one: the code entry takes characters
    /// and Backspace, Enter is the face's main action and Escape is its
    /// `BACK`. The on-screen keys do the same job without one.
    fn keys(&mut self, input: &LobbyInput, stage: Stage, room: Option<&RoomView>) -> Option<LobbyAction> {
        if stage == Stage::Code {
            for c in input.typed.chars() {
                // A pod letter is the operator's and need not come from
                // the room alphabet (`RoomCode::parse`), so typing takes
                // any letter or digit where the key grid offers twenty.
                let c = c.to_ascii_uppercase();
                if c.is_ascii_alphanumeric() {
                    self.push(c);
                }
            }
            if input.backspace {
                self.entry.pop();
                self.entry_note = None;
            }
        }
        if input.escape {
            return Some(self.press(Button::Back, room));
        }
        if input.enter {
            let main = match stage {
                Stage::Start => Button::Host,
                Stage::Code => Button::Confirm,
                Stage::Room => Button::Start,
                Stage::Waiting | Stage::Closed => return None,
            };
            if self.enabled(main, room) {
                return Some(self.press(main, room));
            }
        }
        None
    }

    /// Act on a button, whichever way it was pressed.
    fn press(&mut self, button: Button, room: Option<&RoomView>) -> LobbyAction {
        match button {
            Button::Host => LobbyAction::Host { map: self.map().to_string(), mission: self.mission() },
            Button::Join => {
                self.entry_open = true;
                self.entry.clear();
                self.entry_note = None;
                LobbyAction::None
            }
            Button::Back => {
                // Out of the code entry first, out of the lobby second.
                if self.entry_open && room.is_none() {
                    self.entry_open = false;
                    LobbyAction::None
                } else {
                    LobbyAction::Leave
                }
            }
            Button::MapPrev => self.step_map(-1),
            Button::MapNext => self.step_map(1),
            Button::MissionPrev => self.step_mission(-1),
            Button::MissionNext => self.step_mission(1),
            Button::Key(i) => {
                self.push(CODE_ALPHABET[i as usize % CODE_ALPHABET.len()] as char);
                LobbyAction::None
            }
            Button::Del => {
                self.entry.pop();
                self.entry_note = None;
                LobbyAction::None
            }
            Button::Confirm => match RoomCode::parse(&self.entry) {
                Ok(code) => LobbyAction::Join { code: code.text },
                Err(e) => {
                    self.entry_note = Some(e.to_string());
                    LobbyAction::None
                }
            },
            Button::Ready => LobbyAction::Ready,
            Button::Start => LobbyAction::Start,
            Button::Leave => LobbyAction::Leave,
            Button::Kick(row) => match room.and_then(|r| r.seats.get(row as usize)) {
                Some(seat) => LobbyAction::Kick { seat: seat.seat },
                None => LobbyAction::None,
            },
        }
    }

    /// One more character in the entry, up to a whole code.
    fn push(&mut self, c: char) {
        if self.entry.chars().count() < ROOM_LETTERS + 1 {
            self.entry.push(c);
            self.entry_note = None;
        }
    }

    fn step_map(&mut self, by: isize) -> LobbyAction {
        let n = SHIPPED_MAPS.len();
        self.map = (self.map + n).wrapping_add_signed(by) % n;
        LobbyAction::None
    }

    fn step_mission(&mut self, by: isize) -> LobbyAction {
        let n = MISSIONS.len();
        self.mission = (self.mission + n).wrapping_add_signed(by) % n;
        LobbyAction::None
    }

    /// Encode the room's link, once per link rather than once per frame.
    /// A frame's `update` comes before its `view`, so the picture is
    /// never a link behind.
    fn refresh_qr(&mut self, room: Option<&RoomView>) {
        let Some(url) = room.and_then(|r| r.code.as_deref()).map(|code| rooms::join_url(&self.rooms, code)) else { return };
        if self.qr.as_ref().is_none_or(|(had, _)| *had != url) {
            self.qr = Qr::encode(&url).ok().map(|qr| (url, qr));
        }
    }

    /// What the painter draws this frame.
    pub fn view(&self, room: Option<&RoomView>) -> LobbyView {
        let stage = self.stage(room);
        let code = room.and_then(|r| r.code.clone());
        let join_url = code.as_deref().map(|code| rooms::join_url(&self.rooms, code));
        let seats = room.map(|r| self.seat_rows(r)).unwrap_or_default();
        let more = room.map_or(0, |r| r.seats.len().saturating_sub(LOBBY_SEAT_ROWS));
        LobbyView {
            stage,
            title: self.title(stage, room).to_string(),
            sub: self.sub(stage, room),
            code,
            join_url,
            qr: self.qr.as_ref().filter(|_| stage == Stage::Room).map(|(_, qr)| qr.clone()),
            map: self.map().to_string(),
            mission: self.mission(),
            entry: self.entry.clone(),
            seats,
            more,
            buttons: self.buttons(room).into_iter().map(|b| self.button_view(b, room)).collect(),
        }
    }

    fn title(&self, stage: Stage, room: Option<&RoomView>) -> &'static str {
        match stage {
            Stage::Start => "ONLINE CO-OP",
            Stage::Code => "JOIN A ROOM",
            Stage::Waiting => "REACHING THE ROOM",
            Stage::Closed => "THE ROOM IS GONE",
            Stage::Room if room.is_some_and(|r| r.is_host) => "YOUR ROOM",
            Stage::Room => "IN THE ROOM",
        }
    }

    fn sub(&self, stage: Stage, room: Option<&RoomView>) -> String {
        if let Some(note) = room.and_then(|r| r.note.as_deref()) {
            return note.to_string();
        }
        match stage {
            Stage::Start => "Host a room and share the code, or join one.".into(),
            Stage::Code => self.entry_note.clone().unwrap_or_else(|| "Five characters, from the code you were given.".into()),
            Stage::Waiting => format!("{}...", self.rooms.base()),
            Stage::Closed => "Nothing is listening any more.".into(),
            Stage::Room if room.is_some_and(|r| r.ended.is_some()) => {
                let room = room.expect("the guard found one");
                let outcome = match room.ended.expect("the guard found one") {
                    RoundOutcome::Won => "ROUND WON.",
                    RoundOutcome::Lost => "ROUND LOST.",
                    // The round was cut short rather than played out.
                    RoundOutcome::Playing => "ROUND OVER.",
                };
                let next = if room.is_host { "REMATCH when everyone is ready." } else { "Waiting for the host's rematch." };
                format!("{outcome} {next}")
            }
            Stage::Room if room.is_some_and(|r| r.is_host) => "Scan the code or read it out. START when everyone is ready.".into(),
            Stage::Room => "Waiting for the host to start the round.".into(),
        }
    }

    /// The seat rows, padded out to the slots the panel draws so an empty
    /// room says how much room it has.
    fn seat_rows(&self, room: &RoomView) -> Vec<SeatRow> {
        room.seats
            .iter()
            .take(LOBBY_SEAT_ROWS)
            .map(|seat| {
                let host = seat.seat == room.host_seat;
                SeatRow {
                    seat: seat.seat,
                    slot: format!("P{}", seat.seat + 1),
                    nick: seat.nick.clone(),
                    chassis: TankKind::from_row(seat.chassis as i32).map_or("-", TankKind::name),
                    state: match (seat.connected, host, seat.ready) {
                        (false, _, _) => "AWAY",
                        (_, true, _) => "HOST",
                        (_, _, true) => "READY",
                        _ => "WAITING",
                    },
                    ready: seat.ready,
                    you: Some(seat.seat) == room.seat,
                    host,
                }
            })
            .collect()
    }

    fn button_view(&self, button: Button, room: Option<&RoomView>) -> ButtonView {
        let label = match button {
            Button::Host => "HOST A ROOM".to_string(),
            Button::Join => "JOIN A ROOM".to_string(),
            Button::Back => if self.entry_open && room.is_none() { "BACK" } else { "CLOSE" }.to_string(),
            Button::MapPrev | Button::MissionPrev => "<".to_string(),
            Button::MapNext | Button::MissionNext => ">".to_string(),
            Button::Key(i) => (CODE_ALPHABET[i as usize % CODE_ALPHABET.len()] as char).to_string(),
            Button::Del => "DELETE".to_string(),
            Button::Confirm => "JOIN".to_string(),
            Button::Ready => if room.is_some_and(|r| r.ready) { "READY" } else { "I'M READY" }.to_string(),
            Button::Start => if room.is_some_and(|r| r.ended.is_some()) { "REMATCH" } else { "START" }.to_string(),
            Button::Leave => "LEAVE".to_string(),
            Button::Kick(_) => "KICK".to_string(),
        };
        let accent = matches!(button, Button::Host | Button::Confirm | Button::Start | Button::Ready);
        ButtonView { button, label, enabled: self.enabled(button, room), accent }
    }
}

/// The frame's input for the lobby, filled the same way by `app.rs` and
/// by a test: a pointer already through `View::to_bitmap` and
/// `Layout::to_field`, and the keys where there is a keyboard.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LobbyInput {
    /// Field-space pointer.
    pub pointer: Option<Vec2>,
    pub pressed: bool,
    /// Characters typed this frame.
    pub typed: String,
    pub backspace: bool,
    pub enter: bool,
    pub escape: bool,
}

/// The panel, centred in the field.
pub fn panel_rect(field: Rect) -> Rectangle {
    let x = ((field.w - LOBBY_W) / 2.0).round().max(0.0);
    let y = ((field.h - LOBBY_H) / 2.0).round().max(0.0);
    Rectangle::new(x, y, LOBBY_W, LOBBY_H)
}

/// The content box inside the panel: the margin taken off every side.
pub fn content_rect(field: Rect) -> Rectangle {
    let p = panel_rect(field);
    Rectangle::new(p.x + LOBBY_MARGIN, p.y + LOBBY_MARGIN, LOBBY_W - 2.0 * LOBBY_MARGIN, LOBBY_H - 2.0 * LOBBY_MARGIN)
}

/// The row of action buttons along the content's bottom edge.
fn action_y(field: Rect) -> f32 {
    let c = content_rect(field);
    c.y + c.height - LOBBY_BUTTON_H
}

/// The square the QR is drawn in, top-right of the content box.
pub fn qr_rect(field: Rect) -> Rectangle {
    let c = content_rect(field);
    Rectangle::new(c.x + c.width - LOBBY_QR_BOX, c.y, LOBBY_QR_BOX, LOBBY_QR_BOX)
}

/// The column the seats are listed in, left of the QR.
pub fn seats_rect(field: Rect) -> Rectangle {
    let c = content_rect(field);
    let right = qr_rect(field).x - LOBBY_MARGIN;
    Rectangle::new(c.x, c.y + 24.0, right - c.x, LOBBY_SEAT_ROWS as f32 * LOBBY_SEAT_H)
}

/// One seat's row, `row` from the top of [`seats_rect`].
pub fn seat_row_rect(field: Rect, row: usize) -> Rectangle {
    let s = seats_rect(field);
    Rectangle::new(s.x, s.y + row as f32 * LOBBY_SEAT_H, s.width, LOBBY_SEAT_H)
}

/// The five code boxes, `i` from the left.
pub fn code_box_rect(field: Rect, i: usize) -> Rectangle {
    let c = content_rect(field);
    let span = (ROOM_LETTERS + 1) as f32 * LOBBY_CODE_BOX + ROOM_LETTERS as f32 * LOBBY_CODE_GAP;
    let x = c.x + (c.width - span) / 2.0 + i as f32 * (LOBBY_CODE_BOX + LOBBY_CODE_GAP);
    Rectangle::new(x, c.y + 44.0, LOBBY_CODE_BOX, LOBBY_CODE_BOX)
}

/// **The one geometry table**: where a button is, in field space. The
/// drawing and every hit test read it, so a tool's click and a finger land
/// on the same rect.
pub fn button_rect(field: Rect, button: Button) -> Rectangle {
    let c = content_rect(field);
    let bottom = action_y(field);
    let left = Rectangle::new(c.x, bottom, LOBBY_BUTTON_W, LOBBY_BUTTON_H);
    let middle = Rectangle::new(c.x + LOBBY_BUTTON_W + LOBBY_MARGIN * 0.8, bottom, LOBBY_BUTTON_W, LOBBY_BUTTON_H);
    let right = Rectangle::new(c.x + c.width - LOBBY_BUTTON_W, bottom, LOBBY_BUTTON_W, LOBBY_BUTTON_H);
    // The two steppers of the opening face, one row apart.
    let stepper = |row: f32, next: bool| {
        let y = c.y + 56.0 + row * 60.0;
        let x = if next { c.x + 436.0 } else { c.x + 160.0 };
        Rectangle::new(x, y, LOBBY_BUTTON_H, LOBBY_BUTTON_H)
    };
    match button {
        Button::MapPrev => stepper(0.0, false),
        Button::MapNext => stepper(0.0, true),
        Button::MissionPrev => stepper(1.0, false),
        Button::MissionNext => stepper(1.0, true),
        Button::Host => Rectangle::new(c.x + (c.width - 2.0 * LOBBY_WIDE_W - LOBBY_MARGIN) / 2.0, c.y + 190.0, LOBBY_WIDE_W, LOBBY_BUTTON_H),
        Button::Join => Rectangle::new(
            c.x + (c.width - 2.0 * LOBBY_WIDE_W - LOBBY_MARGIN) / 2.0 + LOBBY_WIDE_W + LOBBY_MARGIN,
            c.y + 190.0,
            LOBBY_WIDE_W,
            LOBBY_BUTTON_H,
        ),
        Button::Key(i) => {
            let i = i as usize % CODE_ALPHABET.len();
            let (row, col) = (i / LOBBY_KEY_COLS, i % LOBBY_KEY_COLS);
            let span = LOBBY_KEY_COLS as f32 * LOBBY_KEY_W + (LOBBY_KEY_COLS - 1) as f32 * LOBBY_KEY_GAP;
            Rectangle::new(
                c.x + (c.width - span) / 2.0 + col as f32 * (LOBBY_KEY_W + LOBBY_KEY_GAP),
                c.y + 120.0 + row as f32 * (LOBBY_KEY_H + LOBBY_KEY_GAP),
                LOBBY_KEY_W,
                LOBBY_KEY_H,
            )
        }
        Button::Del | Button::Ready => left,
        Button::Confirm | Button::Start => middle,
        Button::Back | Button::Leave => right,
        Button::Kick(row) => {
            let r = seat_row_rect(field, row as usize % LOBBY_SEAT_ROWS);
            Rectangle::new(r.x + r.width - LOBBY_KICK_W, r.y, LOBBY_KICK_W, LOBBY_SEAT_H)
        }
    }
}

#[cfg(test)]
mod lobby_tests {
    use super::*;
    use crate::net::wire::{RosterSeat, RoundOutcome};

    const FIELD: Rect = Rect::new(0.0, 32.0, crate::DEFAULT_SCREEN_WIDTH as f32, crate::DEFAULT_SCREEN_HEIGHT as f32);
    /// The smallest field the game ships a map for (`maps/crossplay/`).
    const SMALL: Rect = Rect::new(0.0, 32.0, 24.0 * 32.0, 12.0 * 32.0);

    fn lobby() -> Lobby {
        Lobby::new(RoomsHost::cluster())
    }

    fn seat(n: u8, nick: &str, ready: bool) -> RosterSeat {
        RosterSeat { seat: n, nick: nick.into(), chassis: 3, ready, connected: true }
    }

    /// The same room once its round has been played out.
    fn ended(room: RoomView, outcome: RoundOutcome) -> RoomView {
        RoomView { ended: Some(outcome), ..room }
    }

    fn room(is_host: bool, seats: Vec<RosterSeat>) -> RoomView {
        let can_start = is_host && seats.iter().all(|s| s.ready || s.seat == 0);
        RoomView {
            phase: RoomPhase::InRoom,
            code: Some("AK7QX".into()),
            seat: Some(if is_host { 0 } else { 1 }),
            host_seat: 0,
            is_host,
            can_start,
            ready: false,
            seats,
            note: None,
            ended: None,
        }
    }

    fn centre(r: Rectangle) -> Vec2 {
        Vec2::new(r.x + r.width / 2.0, r.y + r.height / 2.0)
    }

    fn tap(p: Vec2) -> LobbyInput {
        LobbyInput { pointer: Some(p), pressed: true, ..Default::default() }
    }

    /// Every button of every face: finger-sized, inside the panel, and
    /// never on top of another one.
    #[test]
    fn every_button_is_finger_sized_inside_the_panel_and_clear_of_the_rest() {
        let mut lobby = lobby();
        let playing = room(true, vec![seat(0, "oto", false), seat(1, "ana", true)]);
        let rooms = [None, Some(playing.clone()), Some(ended(playing, RoundOutcome::Lost))];
        for field in [FIELD, SMALL] {
            let panel = panel_rect(field);
            assert!(panel.width <= field.w && panel.height <= field.h, "the panel fits the field");
            for open in [false, true] {
                lobby.entry_open = open;
                for room in &rooms {
                    let buttons = lobby.buttons(room.as_ref());
                    let rects: Vec<Rectangle> = buttons.iter().map(|&b| button_rect(field, b)).collect();
                    for (b, r) in buttons.iter().zip(&rects) {
                        assert!(r.width >= LOBBY_TOUCH_MIN && r.height >= LOBBY_TOUCH_MIN, "{b:?} is too small to hit");
                        assert!(r.x >= panel.x && r.x + r.width <= panel.x + panel.width, "{b:?} runs out of the panel");
                        assert!(r.y >= panel.y && r.y + r.height <= panel.y + panel.height, "{b:?} runs out of the panel");
                    }
                    for (i, a) in rects.iter().enumerate() {
                        for b in &rects[i + 1..] {
                            let apart = a.x + a.width <= b.x || b.x + b.width <= a.x || a.y + a.height <= b.y || b.y + b.height <= a.y;
                            assert!(apart, "{:?} overlaps {:?}", buttons[i], a);
                        }
                    }
                }
            }
        }
    }

    /// The pointer path a browser actually takes, end to end. The page
    /// keeps the canvas at the bitmap's own shape and raylib maps a tap
    /// by dividing by the canvas's CSS box, so a tap arrives in window
    /// pixels at whatever scale the page chose - a phone's narrow box,
    /// one bitmap pixel each, or the 1.5x cap on a monitor. `View` and
    /// `Layout` are what carry it back onto the panel, and a button that
    /// is drawn at one place and hit at another is exactly what a fixed
    /// panel in a scaled canvas would go wrong at.
    #[test]
    fn a_tap_on_a_scaled_canvas_lands_on_the_button_it_is_drawn_on() {
        let layout = crate::Layout::for_field(crate::DEFAULT_SCREEN_WIDTH as f32, crate::DEFAULT_SCREEN_HEIGHT as f32);
        let (w, h) = layout.window_size();
        let bitmap = (w as f32, h as f32);
        let mut lobby = lobby();
        let playing = room(true, vec![seat(0, "oto", false), seat(1, "ana", true)]);
        for scale in [0.35_f32, 1.0, 1.5] {
            let view = crate::view::View::fit(bitmap, (bitmap.0 * scale, bitmap.1 * scale));
            for (open, room) in [(false, None), (true, None), (false, Some(&playing))] {
                lobby.entry_open = open;
                for button in lobby.buttons(room) {
                    if !lobby.enabled(button, room) {
                        continue;
                    }
                    let drawn = centre(button_rect(layout.field, button));
                    // Where the page puts that pixel on the canvas, and
                    // the tap on it coming back the other way.
                    let on_canvas = view.to_window(Vec2::new(drawn.x + layout.field.x, drawn.y + layout.field.y));
                    let tapped = layout.to_field(view.to_bitmap(on_canvas));
                    assert_eq!(lobby.hit(layout.field, tapped, room), Some(button), "at {scale}x");
                }
            }
        }
    }

    /// The QR square and the seat column share the panel without meeting,
    /// and neither reaches the buttons.
    #[test]
    fn the_qr_and_the_seats_share_the_panel_without_touching() {
        let qr = qr_rect(FIELD);
        let seats = seats_rect(FIELD);
        assert!(seats.x + seats.width <= qr.x, "the seats run under the QR");
        assert!(qr.width >= 4.0 * 37.0, "a version 3 code needs 148 px at four pixels a module");
        let bottom = button_rect(FIELD, Button::Ready).y;
        assert!(seats.y + seats.height <= bottom, "the seat rows run into the buttons");
        assert!(qr.y + qr.height <= bottom, "the QR runs into the buttons");
        for row in 0..LOBBY_SEAT_ROWS {
            let r = seat_row_rect(FIELD, row);
            let kick = button_rect(FIELD, Button::Kick(row as u8));
            assert!(kick.x + kick.width <= r.x + r.width && kick.y == r.y);
        }
    }

    /// A tap on a key is a character; five of them spell a code; `JOIN`
    /// is dead until they do and then carries the canonical code.
    #[test]
    fn the_code_entry_fills_from_the_on_screen_alphabet_and_gates_join() {
        let mut lobby = lobby();
        assert_eq!(lobby.stage(None), Stage::Start);
        lobby.update(&tap(centre(button_rect(FIELD, Button::Join))), FIELD, None);
        assert_eq!(lobby.stage(None), Stage::Code);
        // `JOIN` and `DELETE` are dead on an empty entry, so a tap on
        // them is nothing at all.
        assert_eq!(lobby.hit(FIELD, centre(button_rect(FIELD, Button::Confirm)), None), None);
        assert_eq!(lobby.hit(FIELD, centre(button_rect(FIELD, Button::Del)), None), None);
        // C D F G H, the first five keys.
        for i in 0..5u8 {
            assert_eq!(lobby.update(&tap(centre(button_rect(FIELD, Button::Key(i)))), FIELD, None), LobbyAction::None);
        }
        assert_eq!(lobby.entry(), "CDFGH");
        // A sixth key is refused: a code is five characters.
        lobby.update(&tap(centre(button_rect(FIELD, Button::Key(5)))), FIELD, None);
        assert_eq!(lobby.entry(), "CDFGH");
        assert_eq!(lobby.update(&tap(centre(button_rect(FIELD, Button::Del))), FIELD, None), LobbyAction::None);
        assert_eq!(lobby.entry(), "CDFG");
        lobby.update(&tap(centre(button_rect(FIELD, Button::Key(4)))), FIELD, None);
        let action = lobby.update(&tap(centre(button_rect(FIELD, Button::Confirm))), FIELD, None);
        assert_eq!(action, LobbyAction::Join { code: "CDFGH".into() });
    }

    /// The keyboard does the same job: characters, Backspace, Enter to
    /// join, Escape back out of the entry. A pod letter the twenty keys do
    /// not carry can still be typed.
    #[test]
    fn the_keyboard_types_a_code_including_a_pod_letter_off_the_alphabet() {
        let mut lobby = lobby();
        lobby.update(&tap(centre(button_rect(FIELD, Button::Join))), FIELD, None);
        let typed = |text: &str| LobbyInput { typed: text.into(), ..Default::default() };
        lobby.update(&typed("ak7qx"), FIELD, None);
        assert_eq!(lobby.entry(), "AK7QX", "upper-cased as it is typed");
        assert_eq!(lobby.update(&LobbyInput { backspace: true, ..Default::default() }, FIELD, None), LobbyAction::None);
        assert_eq!(lobby.entry(), "AK7Q");
        // Enter with an incomplete code does nothing.
        assert_eq!(lobby.update(&LobbyInput { enter: true, ..Default::default() }, FIELD, None), LobbyAction::None);
        lobby.update(&typed("x"), FIELD, None);
        assert_eq!(
            lobby.update(&LobbyInput { enter: true, ..Default::default() }, FIELD, None),
            LobbyAction::Join { code: "AK7QX".into() }
        );
        // Escape steps out of the entry, and then out of the lobby.
        let esc = LobbyInput { escape: true, ..Default::default() };
        assert_eq!(lobby.update(&esc, FIELD, None), LobbyAction::None);
        assert_eq!(lobby.stage(None), Stage::Start);
        assert_eq!(lobby.update(&esc, FIELD, None), LobbyAction::Leave);
    }

    /// The opening face: the map and mission steppers walk the shipped
    /// maps and the three missions, and `HOST` carries what they land on.
    #[test]
    fn the_steppers_pick_the_map_and_mission_the_host_button_sends() {
        let mut lobby = lobby();
        assert_eq!(lobby.map(), SHIPPED_MAPS[0].0);
        lobby.update(&tap(centre(button_rect(FIELD, Button::MapNext))), FIELD, None);
        assert_eq!(lobby.map(), SHIPPED_MAPS[1].0);
        lobby.update(&tap(centre(button_rect(FIELD, Button::MapPrev))), FIELD, None);
        lobby.update(&tap(centre(button_rect(FIELD, Button::MapPrev))), FIELD, None);
        assert_eq!(lobby.map(), SHIPPED_MAPS[SHIPPED_MAPS.len() - 1].0, "the stepper wraps");
        lobby.update(&tap(centre(button_rect(FIELD, Button::MissionNext))), FIELD, None);
        assert_eq!(lobby.mission(), Mission::Hunt);
        let action = lobby.update(&tap(centre(button_rect(FIELD, Button::Host))), FIELD, None);
        assert_eq!(action, LobbyAction::Host { map: SHIPPED_MAPS[SHIPPED_MAPS.len() - 1].0.into(), mission: Mission::Hunt });
    }

    /// The screen offers exactly what the client would take: no `START`
    /// or `KICK` for a guest, no `START` before the room can start, and no
    /// kicking oneself.
    #[test]
    fn start_and_kick_follow_the_clients_own_rules() {
        let mut lobby = lobby();
        let waiting = room(true, vec![seat(0, "oto", false), seat(1, "ana", false)]);
        assert!(!waiting.can_start);
        assert_eq!(lobby.stage(Some(&waiting)), Stage::Room);
        assert!(lobby.buttons(Some(&waiting)).contains(&Button::Start), "the host's button is drawn");
        assert_eq!(lobby.hit(FIELD, centre(button_rect(FIELD, Button::Start)), Some(&waiting)), None, "dead until the room can start");
        assert_eq!(lobby.update(&LobbyInput { enter: true, ..Default::default() }, FIELD, Some(&waiting)), LobbyAction::None);

        let ready = room(true, vec![seat(0, "oto", false), seat(1, "ana", true)]);
        assert!(ready.can_start);
        assert_eq!(lobby.update(&tap(centre(button_rect(FIELD, Button::Start))), FIELD, Some(&ready)), LobbyAction::Start);
        assert_eq!(lobby.update(&LobbyInput { enter: true, ..Default::default() }, FIELD, Some(&ready)), LobbyAction::Start);
        // Row 1 is the other seat; row 0 is the host itself and has no button.
        assert_eq!(lobby.buttons(Some(&ready)).iter().filter(|b| matches!(b, Button::Kick(_))).count(), 1);
        assert_eq!(lobby.update(&tap(centre(button_rect(FIELD, Button::Kick(1)))), FIELD, Some(&ready)), LobbyAction::Kick { seat: 1 });

        let guest = room(false, vec![seat(0, "oto", false), seat(1, "ana", false)]);
        let buttons = lobby.buttons(Some(&guest));
        assert!(!buttons.contains(&Button::Start), "a guest never starts the round");
        assert!(!buttons.iter().any(|b| matches!(b, Button::Kick(_))), "a guest never kicks");
        assert_eq!(lobby.update(&tap(centre(button_rect(FIELD, Button::Ready))), FIELD, Some(&guest)), LobbyAction::Ready);
        assert_eq!(lobby.update(&tap(centre(button_rect(FIELD, Button::Leave))), FIELD, Some(&guest)), LobbyAction::Leave);
    }

    /// The view a painter reads: the room's code, its link, a QR of that
    /// link, and a row per seat saying who is host, who is ready and which
    /// one is you.
    #[test]
    fn the_view_carries_the_code_its_qr_and_a_row_per_seat() {
        let mut lobby = lobby();
        let mut away = seat(2, "kaj", false);
        away.connected = false;
        let room = room(false, vec![seat(0, "oto", false), seat(1, "ana", true), away]);
        lobby.update(&LobbyInput::default(), FIELD, Some(&room));
        let view = lobby.view(Some(&room));
        assert_eq!(view.stage, Stage::Room);
        assert_eq!(view.code.as_deref(), Some("AK7QX"));
        assert_eq!(view.join_url.as_deref(), Some("https://bongbong.io/j/AK7QX"));
        let qr = view.qr.expect("the link has a QR");
        assert_eq!(qr.version(), 2, "27 characters is a version 2 code");
        assert!(qr.padded_size() as f32 * qr.scale_for(LOBBY_QR_BOX as i32) as f32 <= LOBBY_QR_BOX);
        let states: Vec<&str> = view.seats.iter().map(|s| s.state).collect();
        assert_eq!(states, vec!["HOST", "READY", "AWAY"]);
        assert_eq!(view.seats[1].nick, "ana");
        assert!(view.seats[1].you && !view.seats[0].you);
        assert_eq!(view.seats[0].chassis, "longbow", "the roster's chassis row");
        assert_eq!(view.more, 0);
        // A refusal from the room is the line under the title.
        let refused = RoomView { note: Some("the room is full".into()), ..room };
        assert_eq!(lobby.view(Some(&refused)).sub, "the room is full");
    }

    /// A local override rides along in the link, so a scan reaches the
    /// same server, and the longer link is still one QR.
    #[test]
    fn a_local_rooms_override_travels_in_the_link_and_its_qr() {
        let mut lobby = Lobby::new(RoomsHost::overriding("ws://127.0.0.1:4848"));
        let room = room(true, vec![seat(0, "oto", false)]);
        lobby.update(&LobbyInput::default(), FIELD, Some(&room));
        let view = lobby.view(Some(&room));
        assert_eq!(view.join_url.as_deref(), Some("https://bongbong.io/j/AK7QX?rooms=ws://127.0.0.1:4848"));
        assert_eq!(view.qr.expect("encodes").version(), 3);
    }

    /// The room face once the round is over: the outcome under the seats,
    /// the host's action reading `REMATCH` rather than `START`, and the
    /// guest told whose move it is. Nothing else about the face moves -
    /// the code, the QR and the seats are the ones the round was played
    /// on - and `LEAVE` still leaves.
    #[test]
    fn the_room_face_reads_as_an_end_screen_once_the_round_is_over() {
        let mut lobby = lobby();
        // The room clears every seat's ready when a round starts, so a
        // rematch is asked for the way the first round was.
        let seats = vec![seat(0, "oto", false), seat(1, "ana", true)];
        let host = ended(room(true, seats.clone()), RoundOutcome::Won);
        lobby.update(&LobbyInput::default(), FIELD, Some(&host));
        let view = lobby.view(Some(&host));
        assert_eq!(view.stage, Stage::Room, "the room outlives its round");
        assert_eq!(view.code.as_deref(), Some("AK7QX"));
        assert!(view.qr.is_some(), "the invite is still up for anyone rejoining");
        assert_eq!(view.seats.len(), 2);
        assert_eq!(view.sub, "ROUND WON. REMATCH when everyone is ready.");
        let action = |view: &LobbyView| {
            view.buttons.iter().find(|b| b.button == Button::Start).map(|b| (b.label.clone(), b.enabled))
        };
        assert_eq!(action(&view), Some(("REMATCH".to_string(), true)));
        // Dead until the other seat says it is ready again, exactly as
        // `START` is before the first round.
        let waiting = ended(room(true, vec![seat(0, "oto", false), seat(1, "ana", false)]), RoundOutcome::Won);
        assert_eq!(action(&lobby.view(Some(&waiting))), Some(("REMATCH".to_string(), false)));
        assert_eq!(lobby.hit(FIELD, centre(button_rect(FIELD, Button::Start)), Some(&waiting)), None);
        // The rects are the ones a finger already knows: a relabelled
        // button is the same button.
        assert_eq!(button_rect(FIELD, Button::Start), button_rect(FIELD, Button::Confirm));
        assert_eq!(lobby.update(&tap(centre(button_rect(FIELD, Button::Start))), FIELD, Some(&host)), LobbyAction::Start);
        assert_eq!(lobby.update(&tap(centre(button_rect(FIELD, Button::Leave))), FIELD, Some(&host)), LobbyAction::Leave);

        // A lost round says so, and a guest is told whose move it is.
        let lost = ended(room(true, seats.clone()), RoundOutcome::Lost);
        assert!(lobby.view(Some(&lost)).sub.starts_with("ROUND LOST."), "{}", lobby.view(Some(&lost)).sub);
        let guest = ended(room(false, seats), RoundOutcome::Won);
        let view = lobby.view(Some(&guest));
        assert_eq!(view.sub, "ROUND WON. Waiting for the host's rematch.");
        assert_eq!(action(&view), None, "a guest never starts the round, rematch or not");
        assert!(view.buttons.iter().any(|b| b.button == Button::Leave));
    }

    /// A roster longer than the rows says how many it left out, and only
    /// the rows it draws are kickable.
    #[test]
    fn a_full_room_says_how_many_seats_it_left_out() {
        let mut lobby = lobby();
        let seats: Vec<RosterSeat> = (0..crate::MAX_SEATS as u8).map(|i| seat(i, "p", i > 0)).collect();
        let room = room(true, seats);
        lobby.update(&LobbyInput::default(), FIELD, Some(&room));
        let view = lobby.view(Some(&room));
        assert_eq!(view.seats.len(), LOBBY_SEAT_ROWS);
        assert_eq!(view.more, crate::MAX_SEATS - LOBBY_SEAT_ROWS);
        // Each row is named for its seat, and reads as what that seat is.
        assert_eq!(view.seats.iter().map(|s| s.slot.as_str()).collect::<Vec<_>>(), vec!["P1", "P2", "P3", "P4"]);
        assert_eq!(view.seats[0].state, "HOST");
        assert!(view.seats[1..].iter().all(|s| s.state == "READY"));
        // A full room starts once everybody but the host has readied,
        // and every kick button belongs to a row that is drawn.
        assert!(room.can_start);
        let kicks: Vec<u8> = lobby
            .buttons(Some(&room))
            .into_iter()
            .filter_map(|b| match b {
                Button::Kick(row) => Some(row),
                _ => None,
            })
            .collect();
        assert_eq!(kicks, vec![1, 2, 3], "the host's own row has no kick, and no row past the panel's");
        let column = seats_rect(FIELD);
        for row in kicks {
            let r = button_rect(FIELD, Button::Kick(row));
            assert!(r.y >= column.y && r.y + r.height <= column.y + column.height);
        }
    }
}
