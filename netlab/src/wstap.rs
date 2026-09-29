//! The wire tap: one direction of a WebSocket connection read back into
//! messages as the proxy passes the bytes through.
//!
//! The stream starts with the HTTP upgrade (request or response headers,
//! up to the first blank line), then RFC 6455 frames: FIN and the opcode,
//! the mask bit, a 7-, 16- or 64-bit length, a masking key on frames the
//! client sends, the payload. Fragments are joined into their message;
//! control frames (ping, pong, close) are handed over on their own and may
//! arrive between a message's fragments.
//!
//! Bytes arrive in whatever chunks the socket read them in, so a frame can
//! span several chunks and a chunk hold several frames. Each message is
//! stamped with the chunk that completed it: the proxy passes in when that
//! chunk reached it and when it left.

/// A whole message off the stream.
#[derive(Clone, Debug, PartialEq)]
pub struct WsMessage {
    /// The first frame's opcode: 1 text, 2 binary, 8 close, 9 ping, 10 pong.
    pub opcode: u8,
    pub payload: Vec<u8>,
    /// When the chunk that completed it reached the proxy.
    pub ingress_ms: f64,
    /// When that chunk left the proxy.
    pub egress_ms: f64,
}

/// The binary opcode.
pub const OP_BINARY: u8 = 2;

/// Why a stream stopped making sense. The tap stops reading that
/// direction; the proxy keeps passing its bytes through regardless.
#[derive(Clone, Debug, PartialEq)]
pub enum TapError {
    /// A frame claimed a length past what any message of this protocol
    /// could be.
    Oversized(u64),
    /// A continuation with no message open, or a new data frame while one
    /// was.
    Fragmentation,
}

/// The largest frame the tap will buffer: well past any snapshot or
/// welcome the room sends.
pub const MAX_FRAME: u64 = 16 << 20;

/// One direction's parser.
#[derive(Debug, Default)]
pub struct WsTap {
    headers_done: bool,
    buf: Vec<u8>,
    /// A fragmented data message being joined: its opcode and payload.
    partial: Option<(u8, Vec<u8>)>,
    failed: Option<TapError>,
}

impl WsTap {
    pub fn new() -> WsTap {
        WsTap::default()
    }

    /// A tap on a stream that carries no upgrade headers (tests).
    pub fn frames_only() -> WsTap {
        WsTap { headers_done: true, ..WsTap::default() }
    }

    /// What stopped the tap, if anything did.
    pub fn failed(&self) -> Option<&TapError> {
        self.failed.as_ref()
    }

    /// Feed one chunk; every message it completes is appended to `out`.
    pub fn feed(&mut self, bytes: &[u8], ingress_ms: f64, egress_ms: f64, out: &mut Vec<WsMessage>) {
        if self.failed.is_some() {
            return;
        }
        self.buf.extend_from_slice(bytes);
        if !self.headers_done {
            let Some(end) = self.buf.windows(4).position(|w| w == b"\r\n\r\n") else { return };
            self.buf.drain(..end + 4);
            self.headers_done = true;
        }
        loop {
            match parse_frame(&self.buf) {
                Ok(Some((frame, used))) => {
                    self.buf.drain(..used);
                    if let Err(e) = self.take(frame, ingress_ms, egress_ms, out) {
                        self.failed = Some(e);
                        return;
                    }
                }
                Ok(None) => return,
                Err(e) => {
                    self.failed = Some(e);
                    return;
                }
            }
        }
    }

    fn take(&mut self, frame: Frame, ingress_ms: f64, egress_ms: f64, out: &mut Vec<WsMessage>) -> Result<(), TapError> {
        let emit = |opcode, payload| WsMessage { opcode, payload, ingress_ms, egress_ms };
        if frame.opcode >= 8 {
            out.push(emit(frame.opcode, frame.payload));
            return Ok(());
        }
        match (frame.opcode, self.partial.take()) {
            (0, Some((op, mut payload))) => {
                payload.extend_from_slice(&frame.payload);
                if frame.fin {
                    out.push(emit(op, payload));
                } else {
                    self.partial = Some((op, payload));
                }
                Ok(())
            }
            (0, None) | (_, Some(_)) => Err(TapError::Fragmentation),
            (op, None) => {
                if frame.fin {
                    out.push(emit(op, frame.payload));
                } else {
                    self.partial = Some((op, frame.payload));
                }
                Ok(())
            }
        }
    }
}

/// One frame, unmasked.
#[derive(Clone, Debug, PartialEq)]
struct Frame {
    fin: bool,
    opcode: u8,
    payload: Vec<u8>,
}

/// The frame at the head of `buf` and the bytes it used, or `None` while
/// it is still incomplete.
fn parse_frame(buf: &[u8]) -> Result<Option<(Frame, usize)>, TapError> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let fin = buf[0] & 0x80 != 0;
    let opcode = buf[0] & 0x0F;
    let masked = buf[1] & 0x80 != 0;
    let (len, mut at) = match buf[1] & 0x7F {
        126 => {
            if buf.len() < 4 {
                return Ok(None);
            }
            (u16::from_be_bytes([buf[2], buf[3]]) as u64, 4)
        }
        127 => {
            if buf.len() < 10 {
                return Ok(None);
            }
            let mut b = [0u8; 8];
            b.copy_from_slice(&buf[2..10]);
            (u64::from_be_bytes(b), 10)
        }
        n => (n as u64, 2),
    };
    if len > MAX_FRAME {
        return Err(TapError::Oversized(len));
    }
    let key = if masked {
        if buf.len() < at + 4 {
            return Ok(None);
        }
        let k = [buf[at], buf[at + 1], buf[at + 2], buf[at + 3]];
        at += 4;
        Some(k)
    } else {
        None
    };
    let end = at + len as usize;
    if buf.len() < end {
        return Ok(None);
    }
    let mut payload = buf[at..end].to_vec();
    if let Some(k) = key {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= k[i % 4];
        }
    }
    Ok(Some((Frame { fin, opcode, payload }, end)))
}

/// A frame as a peer would write it: what the tests feed the tap, and a
/// reference for the layout.
pub fn encode_frame(fin: bool, opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = vec![if fin { 0x80 } else { 0 } | (opcode & 0x0F)];
    let mask_bit = if mask.is_some() { 0x80 } else { 0 };
    match payload.len() {
        n if n < 126 => out.push(mask_bit | n as u8),
        n if n <= u16::MAX as usize => {
            out.push(mask_bit | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(mask_bit | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    match mask {
        Some(k) => {
            out.extend_from_slice(&k);
            out.extend(payload.iter().enumerate().map(|(i, b)| b ^ k[i % 4]));
        }
        None => out.extend_from_slice(payload),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed_all(tap: &mut WsTap, bytes: &[u8]) -> Vec<WsMessage> {
        let mut out = Vec::new();
        tap.feed(bytes, 1.0, 2.0, &mut out);
        out
    }

    #[test]
    fn an_unmasked_short_frame_after_the_upgrade_headers() {
        let mut tap = WsTap::new();
        let mut stream = b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n".to_vec();
        stream.extend(encode_frame(true, OP_BINARY, b"hello", None));
        let out = feed_all(&mut tap, &stream);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].opcode, OP_BINARY);
        assert_eq!(out[0].payload, b"hello");
        assert_eq!((out[0].ingress_ms, out[0].egress_ms), (1.0, 2.0));
    }

    #[test]
    fn a_masked_frame_is_unmasked() {
        let mut tap = WsTap::frames_only();
        let payload: Vec<u8> = (0..100u8).collect();
        let out = feed_all(&mut tap, &encode_frame(true, OP_BINARY, &payload, Some([0xA1, 0x07, 0x5C, 0xE3])));
        assert_eq!(out[0].payload, payload);
    }

    #[test]
    fn sixteen_and_sixty_four_bit_lengths() {
        for len in [126usize, 300, 65_535, 65_536, 200_000] {
            for mask in [None, Some([1, 2, 3, 4])] {
                let payload: Vec<u8> = (0..len).map(|i| (i * 7 % 251) as u8).collect();
                let frame = encode_frame(true, OP_BINARY, &payload, mask);
                let expected_header = if len <= 65_535 { 4 } else { 10 } + if mask.is_some() { 4 } else { 0 };
                assert_eq!(frame.len(), expected_header + len);
                let mut tap = WsTap::frames_only();
                let out = feed_all(&mut tap, &frame);
                assert_eq!(out.len(), 1, "len {len} mask {mask:?}");
                assert_eq!(out[0].payload, payload);
            }
        }
    }

    #[test]
    fn a_frame_split_across_chunks_is_stamped_with_the_chunk_that_completed_it() {
        let mut tap = WsTap::frames_only();
        let frame = encode_frame(true, OP_BINARY, &[9u8; 1000], Some([9, 8, 7, 6]));
        let mut out = Vec::new();
        for (i, chunk) in frame.chunks(7).enumerate() {
            tap.feed(chunk, i as f64, i as f64 + 0.5, &mut out);
        }
        assert_eq!(out.len(), 1);
        let last = (frame.len().div_ceil(7) - 1) as f64;
        assert_eq!(out[0].ingress_ms, last);
        assert_eq!(out[0].payload, vec![9u8; 1000]);
    }

    #[test]
    fn several_frames_in_one_chunk_and_fragments_joined_around_a_ping() {
        let mut tap = WsTap::frames_only();
        let mut stream = encode_frame(true, OP_BINARY, b"one", None);
        stream.extend(encode_frame(false, OP_BINARY, b"tw", None));
        stream.extend(encode_frame(true, 9, b"p", None));
        stream.extend(encode_frame(true, 0, b"o", None));
        let out = feed_all(&mut tap, &stream);
        let got: Vec<(u8, &[u8])> = out.iter().map(|m| (m.opcode, m.payload.as_slice())).collect();
        assert_eq!(got, vec![(OP_BINARY, &b"one"[..]), (9, &b"p"[..]), (OP_BINARY, &b"two"[..])]);
    }

    #[test]
    fn a_stray_continuation_stops_the_tap() {
        let mut tap = WsTap::frames_only();
        let out = feed_all(&mut tap, &encode_frame(true, 0, b"x", None));
        assert!(out.is_empty());
        assert_eq!(tap.failed(), Some(&TapError::Fragmentation));
    }
}
