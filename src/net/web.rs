//! The web socket (docs/online-coop-prd.md §4.6): emscripten's own
//! WebSocket API (`emscripten/websocket.h`, linked with
//! `-lwebsocket.js` from `.cargo/config.toml`) through FFI. No site
//! JavaScript, no `--js-library` file, no Rust socket code and no
//! threads - the browser owns the connection and hands it back through
//! four callbacks.
//!
//! Those callbacks land between frames: the page's frame is a
//! `requestAnimationFrame` task and the socket's events are tasks of
//! their own, so one can never interrupt the other. Each pushes into a
//! queue and returns; the frame drains it at its boundary, the same
//! discipline `tuning::apply_pending` and the dev server follow. Nothing
//! blocks and nothing locks.
//!
//! The queue is deliberately unbounded. A hidden tab stops rAF while its
//! socket keeps delivering, so the backlog is the price of coming back:
//! the frame that returns drains all of it and the delta chain steps
//! forward message by message. Dropping the middle of that backlog would
//! break the chain, which costs far more than the memory - a backlog of
//! a minute is a few hundred snapshots.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::ffi::{CString, c_char, c_int, c_void};
use std::rc::Rc;

use crate::net::codec::{self, Msg};
use crate::net::transport::{Closed, ConnState, Transport};

// ---------------------------------------------------------------------------
// emscripten/websocket.h

/// `EMSCRIPTEN_WEBSOCKET_T`: the socket handle, positive when made.
type SocketHandle = c_int;

/// `EM_CALLBACK_THREAD_CONTEXT_CALLING_THREAD`: deliver the callbacks on
/// the thread that made the socket, which here is the only one there is.
const CALLING_THREAD: *mut c_void = 0x2 as *mut c_void;

/// `EMSCRIPTEN_RESULT_SUCCESS`.
const RESULT_SUCCESS: c_int = 0;

/// The close code sent when this end hangs up: 1000, a normal closure.
const CLOSE_NORMAL: u16 = 1000;

#[repr(C)]
struct CreateAttributes {
    url: *const c_char,
    protocols: *const c_char,
    create_on_main_thread: bool,
}

#[repr(C)]
struct OpenEvent {
    socket: SocketHandle,
}

#[repr(C)]
struct MessageEvent {
    socket: SocketHandle,
    /// The bytes, valid only for the length of the callback: emscripten
    /// frees them the moment it returns, so they are copied on the spot.
    data: *mut u8,
    num_bytes: u32,
    is_text: bool,
}

#[repr(C)]
struct ErrorEvent {
    socket: SocketHandle,
}

#[repr(C)]
struct CloseEvent {
    socket: SocketHandle,
    was_clean: bool,
    code: u16,
    /// The spec caps a close reason at 123 characters, so the struct
    /// carries a fixed 512-byte buffer for its UTF-8.
    reason: [c_char; 512],
}

type OpenCallback = extern "C" fn(c_int, *const OpenEvent, *mut c_void) -> bool;
type MessageCallback = extern "C" fn(c_int, *const MessageEvent, *mut c_void) -> bool;
type ErrorCallback = extern "C" fn(c_int, *const ErrorEvent, *mut c_void) -> bool;
type CloseCallback = extern "C" fn(c_int, *const CloseEvent, *mut c_void) -> bool;

unsafe extern "C" {
    fn emscripten_websocket_is_supported() -> bool;
    fn emscripten_websocket_new(attributes: *const CreateAttributes) -> SocketHandle;
    fn emscripten_websocket_send_binary(socket: SocketHandle, data: *const u8, len: u32) -> c_int;
    fn emscripten_websocket_close(socket: SocketHandle, code: u16, reason: *const c_char) -> c_int;
    fn emscripten_websocket_delete(socket: SocketHandle) -> c_int;
    fn emscripten_websocket_set_onopen_callback_on_thread(
        socket: SocketHandle,
        user_data: *mut c_void,
        callback: OpenCallback,
        thread: *mut c_void,
    ) -> c_int;
    fn emscripten_websocket_set_onmessage_callback_on_thread(
        socket: SocketHandle,
        user_data: *mut c_void,
        callback: MessageCallback,
        thread: *mut c_void,
    ) -> c_int;
    fn emscripten_websocket_set_onerror_callback_on_thread(
        socket: SocketHandle,
        user_data: *mut c_void,
        callback: ErrorCallback,
        thread: *mut c_void,
    ) -> c_int;
    fn emscripten_websocket_set_onclose_callback_on_thread(
        socket: SocketHandle,
        user_data: *mut c_void,
        callback: CloseCallback,
        thread: *mut c_void,
    ) -> c_int;
}

// ---------------------------------------------------------------------------
// The transport

/// What the callbacks write and the frame reads. One thread touches it,
/// and never at the same moment: the frame's `RefCell` borrow is dropped
/// before it yields, and a callback's before it returns.
struct Inner {
    arrived: VecDeque<Vec<u8>>,
    state: ConnState,
}

/// A WebSocket to a room, run by the browser.
pub struct WebTransport {
    socket: SocketHandle,
    inner: Rc<RefCell<Inner>>,
    /// The same `Rc` as a raw pointer, the `userData` every callback is
    /// handed. Reclaimed in `Drop` once the callbacks can no longer run.
    user_data: *const RefCell<Inner>,
}

impl WebTransport {
    /// Dial `url` (`ws://` or `wss://`, from `rooms::socket_url`).
    ///
    /// Returns as soon as the browser has the socket, which is long
    /// before it is open: the state is `Connecting` until the open
    /// callback lands, and bytes handed to `send` meanwhile are dropped
    /// (the browser's `send` throws before `OPEN`), so a caller greets
    /// the room on the frame it sees `Open`.
    pub fn connect(url: &str) -> Result<WebTransport, String> {
        // SAFETY: a plain query into emscripten's JS glue.
        if !unsafe { emscripten_websocket_is_supported() } {
            return Err("this browser has no WebSocket".into());
        }
        let c_url = CString::new(url).map_err(|_| format!("{url} is not a URL"))?;
        let attributes = CreateAttributes {
            url: c_url.as_ptr(),
            // No sub-protocol: the kind byte in `net::codec` says what a
            // message is.
            protocols: std::ptr::null(),
            // The socket belongs to the thread that made it, which is
            // the one the frame runs on; the build has no pthreads.
            create_on_main_thread: false,
        };
        // SAFETY: `attributes` outlives the call and emscripten reads
        // the URL immediately, as its header states.
        let socket = unsafe { emscripten_websocket_new(&attributes) };
        if socket <= 0 {
            return Err(refusal(url, socket));
        }

        let inner = Rc::new(RefCell::new(Inner {
            arrived: VecDeque::new(),
            state: ConnState::Connecting,
        }));
        // The callbacks hold a reference for as long as the socket
        // lives; `Drop` takes it back.
        let user_data = Rc::into_raw(Rc::clone(&inner));
        let user = user_data as *mut c_void;
        // SAFETY: the socket is live and `user` points at an `Rc` this
        // transport keeps alive until after the socket is deleted.
        unsafe {
            emscripten_websocket_set_onopen_callback_on_thread(socket, user, on_open, CALLING_THREAD);
            emscripten_websocket_set_onmessage_callback_on_thread(socket, user, on_message, CALLING_THREAD);
            emscripten_websocket_set_onerror_callback_on_thread(socket, user, on_error, CALLING_THREAD);
            emscripten_websocket_set_onclose_callback_on_thread(socket, user, on_close, CALLING_THREAD);
        }
        Ok(WebTransport { socket, inner, user_data })
    }
}

impl Transport for WebTransport {
    fn state(&self) -> ConnState {
        self.inner.borrow().state.clone()
    }

    fn send(&mut self, bytes: &[u8]) {
        if self.inner.borrow().state != ConnState::Open || bytes.is_empty() {
            return;
        }
        // SAFETY: the browser copies the bytes out of the heap during
        // the call, as `emscripten_websocket_send_binary` documents.
        let result = unsafe {
            emscripten_websocket_send_binary(self.socket, bytes.as_ptr(), bytes.len() as u32)
        };
        if result != RESULT_SUCCESS {
            let mut inner = self.inner.borrow_mut();
            inner.state = ConnState::Closed(Closed::fault(format!(
                "the browser refused to send to the room (code {result})"
            )));
        }
    }

    fn drain(&mut self, out: &mut Vec<Msg>) {
        let mut inner = self.inner.borrow_mut();
        while let Some(bytes) = inner.arrived.pop_front() {
            if let Ok(msg) = codec::decode(&bytes) {
                out.push(msg);
            }
        }
    }

    fn close(&mut self) {
        {
            let mut inner = self.inner.borrow_mut();
            if matches!(inner.state, ConnState::Closed(_)) {
                return;
            }
            inner.state = ConnState::Closed(Closed::by_us());
        }
        // SAFETY: a live handle and a null reason, which emscripten
        // reads as "no reason given".
        unsafe { emscripten_websocket_close(self.socket, CLOSE_NORMAL, std::ptr::null()) };
    }
}

impl Drop for WebTransport {
    fn drop(&mut self) {
        // SAFETY: closing and deleting the socket is what stops the
        // callbacks, so the reference they were handed is only taken
        // back afterwards.
        unsafe {
            emscripten_websocket_close(self.socket, CLOSE_NORMAL, std::ptr::null());
            emscripten_websocket_delete(self.socket);
            drop(Rc::from_raw(self.user_data));
        }
    }
}

/// What to show a player when the browser would not make the socket at
/// all. It never says why - `new WebSocket` throws and emscripten hands
/// back a negative handle - so the one refusal somebody will actually
/// meet is named: a page served over TLS may open no plain socket, which
/// is what a `?rooms=ws://...` override carried onto an https page comes
/// to (`rooms::Invite`).
fn refusal(url: &str, code: SocketHandle) -> String {
    let refused = format!("the browser refused a socket to {url} (code {code})");
    match url.starts_with("ws://") {
        true => format!("{refused}; a page served over https can only open wss://"),
        false => refused,
    }
}

/// The `Inner` a callback was handed, for the length of the callback.
///
/// # Safety
///
/// `user_data` must be the pointer `connect` made from an `Rc` that is
/// still alive, which holds until `Drop` has deleted the socket.
unsafe fn with_inner(user_data: *mut c_void, f: impl FnOnce(&mut Inner)) -> bool {
    if user_data.is_null() {
        return true;
    }
    let inner = unsafe { &*(user_data as *const RefCell<Inner>) };
    f(&mut inner.borrow_mut());
    true
}

extern "C" fn on_open(_kind: c_int, _event: *const OpenEvent, user_data: *mut c_void) -> bool {
    // SAFETY: the pointer is the transport's, alive until it is dropped.
    unsafe {
        with_inner(user_data, |inner| {
            if !matches!(inner.state, ConnState::Closed(_)) {
                inner.state = ConnState::Open;
            }
        })
    }
}

extern "C" fn on_message(_kind: c_int, event: *const MessageEvent, user_data: *mut c_void) -> bool {
    // SAFETY: emscripten hands a live event for the length of the call
    // and frees its buffer right after, so the bytes are copied here.
    let bytes = unsafe {
        let event = &*event;
        if event.is_text || event.data.is_null() || event.num_bytes == 0 {
            return true;
        }
        std::slice::from_raw_parts(event.data, event.num_bytes as usize).to_vec()
    };
    // SAFETY: as in `on_open`.
    unsafe { with_inner(user_data, move |inner| inner.arrived.push_back(bytes)) }
}

extern "C" fn on_error(_kind: c_int, _event: *const ErrorEvent, user_data: *mut c_void) -> bool {
    // The browser never says what went wrong (the page it would leak to
    // is not the socket's), and a close always follows, which is where
    // the code and the reason are.
    // SAFETY: as in `on_open`.
    unsafe {
        with_inner(user_data, |inner| {
            if !matches!(inner.state, ConnState::Closed(_)) {
                inner.state = ConnState::Closed(Closed::fault("the connection to the room failed"));
            }
        })
    }
}

extern "C" fn on_close(_kind: c_int, event: *const CloseEvent, user_data: *mut c_void) -> bool {
    // SAFETY: a live event for the length of the call; the reason is a
    // NUL-terminated UTF-8 string inside its own buffer.
    let (code, reason) = unsafe {
        let event = &*event;
        let bytes = &event.reason;
        let len = bytes.iter().position(|&c| c == 0).unwrap_or(bytes.len());
        let text = std::str::from_utf8(std::slice::from_raw_parts(bytes.as_ptr() as *const u8, len))
            .unwrap_or("")
            .to_string();
        (event.code, text)
    };
    // SAFETY: as in `on_open`.
    unsafe {
        with_inner(user_data, |inner| {
            if matches!(inner.state, ConnState::Closed(_)) {
                return;
            }
            inner.state = ConnState::Closed(if reason.is_empty() {
                Closed::fault(format!("the room closed the connection (code {code})"))
            } else {
                Closed::fault(reason)
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The structs the callbacks read are emscripten's, laid out by
    /// `emscripten/websocket.h`. Nothing here can be checked against a
    /// running browser from a test, so what is checked is the shape: a
    /// field added, reordered or resized on either side moves these
    /// numbers.
    #[test]
    fn the_event_structs_match_the_header() {
        assert_eq!(size_of::<SocketHandle>(), 4, "EMSCRIPTEN_WEBSOCKET_T is an int");
        assert_eq!(size_of::<OpenEvent>(), 4);
        assert_eq!(size_of::<CloseEvent>(), 4 + 4 + 512, "the handle, the flag and code, the 512-byte reason");
        assert_eq!(size_of::<MessageEvent>(), 16, "handle, pointer, length, flag");
        assert_eq!(size_of::<CreateAttributes>(), 12, "two pointers and a flag");
        // The socket's entry point, named so the linker keeps it and
        // the `emscripten_websocket_*` imports with it: a build that
        // forgot `-lwebsocket.js` fails here rather than in a browser.
        let connect: fn(&str) -> Result<WebTransport, String> = WebTransport::connect;
        assert!(!std::ptr::fn_addr_eq(connect, (|_| Err(String::new())) as fn(&str) -> Result<WebTransport, String>));
    }
}
